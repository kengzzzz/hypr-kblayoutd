use std::io::BufRead;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;

use hypr_kblayoutd::config;
use hypr_kblayoutd::event;
use hypr_kblayoutd::ipc::{HyprlandIpc, HyprlandPaths};
use hypr_kblayoutd::single_instance::SingleInstance;
use hypr_kblayoutd::state::{Action, RuntimeState};

const LAYER_SNAPSHOT_RETRIES: usize = 9;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    if let Err(err) = run() {
        eprintln!("hypr-kblayoutd: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let paths = HyprlandPaths::discover()?;
    let _instance = SingleInstance::acquire(&format!("hypr-kblayoutd-{}", paths.signature))?;
    let config = config::load_default()?;
    let ipc = HyprlandIpc::new(paths);

    let layout_count = ipc.configured_layout_count()?;
    if layout_count < 2 && !ipc.kb_file_is_set()? {
        return Err("Hyprland needs at least two configured keyboard layouts".into());
    }

    let (initial_keyboard, initial_layout) = ipc.current_active_keyboard()?;
    let mut state = RuntimeState::new(config, initial_layout);
    state.seed_keyboard(&initial_keyboard);
    listen_forever(&ipc, &mut state)
}

fn listen_forever(
    ipc: &HyprlandIpc,
    state: &mut RuntimeState,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut backoff = Duration::from_millis(100);

    loop {
        let result = UnixStream::connect(&ipc.paths().event_socket).and_then(|stream| {
            // Connect first so events emitted during the snapshot are buffered.
            resync_state(ipc, state);
            listen_once(stream, ipc, state)
        });
        match result {
            Ok(()) => log::warn!("Hyprland event socket closed; reconnecting"),
            Err(err) => log::warn!("Hyprland event socket error: {err}; reconnecting"),
        }

        thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

// Events can be missed at startup and while the event socket is down, so
// re-align window/layout state with Hyprland before (re)attaching.
fn resync_state(ipc: &HyprlandIpc, state: &mut RuntimeState) {
    let queried = ipc.current_active_layout().and_then(|layout| {
        let active = ipc.active_window()?;
        let clients = ipc.client_addresses()?;
        let layers = ipc.mapped_layer_namespaces()?;
        Ok((layout, active, clients, layers))
    });

    match queried {
        Ok((layout, active, clients, layers)) => {
            let actions = state.resync(active, layout, &clients, &layers);
            run_actions(ipc, state, actions);
        }
        Err(err) => log::warn!("state resync skipped: {err}"),
    }
}

fn listen_once(
    stream: UnixStream,
    ipc: &HyprlandIpc,
    state: &mut RuntimeState,
) -> std::io::Result<()> {
    let mut reader = std::io::BufReader::new(stream);
    let mut line = String::new();

    loop {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Ok(());
        }

        match event::parse_line(&line) {
            Ok(parsed) => {
                let actions = handle_event_with_layer_snapshot(
                    state,
                    parsed,
                    || ipc.mapped_layer_namespaces(),
                    || thread::sleep(Duration::from_millis(2)),
                );
                run_actions(ipc, state, actions);
            }
            Err(err) => log::debug!("ignored malformed Hyprland event {line:?}: {err:?}"),
        }
    }
}

fn handle_event_with_layer_snapshot<E>(
    state: &mut RuntimeState,
    event: event::Event<'_>,
    mut query_layers: impl FnMut() -> Result<Vec<String>, E>,
    mut wait_before_retry: impl FnMut(),
) -> Vec<Action>
where
    E: std::fmt::Display,
{
    let (namespace, opening) = match &event {
        event::Event::OpenLayer { namespace } => (*namespace, true),
        event::Event::CloseLayer { namespace } => (*namespace, false),
        _ => return state.handle_event(event),
    };
    if !state.should_reconcile_layer_event(namespace) {
        return state.handle_event(event);
    }

    let current = state.focused_layer_count(namespace);
    let expected = if opening {
        current.saturating_add(1)
    } else {
        current.saturating_sub(1)
    };
    let snapshot_settled = |layers: &[String]| {
        let observed = layers
            .iter()
            .filter(|known| known.as_str() == namespace)
            .count();
        if opening {
            observed >= expected
        } else {
            observed <= expected
        }
    };
    let mut latest = match query_layers() {
        Ok(layers) => layers,
        Err(err) => {
            log::warn!("layer snapshot failed after event; applying event directly: {err}");
            return state.handle_event(event);
        }
    };
    let mut settled = snapshot_settled(&latest);

    for _ in 0..LAYER_SNAPSHOT_RETRIES {
        if settled {
            break;
        }
        wait_before_retry();
        latest = match query_layers() {
            Ok(layers) => layers,
            Err(err) => {
                log::warn!("layer snapshot failed after event; applying event directly: {err}");
                return state.handle_event(event);
            }
        };
        settled = snapshot_settled(&latest);
    }

    if !settled {
        if opening && current > 0 {
            log::warn!(
                "layer snapshot did not reach repeated open event delta; keeping current count: namespace={namespace}"
            );
        } else {
            log::warn!(
                "layer snapshot did not reach event delta; applying logical event: namespace={namespace} opening={opening}"
            );
        }
    }
    let target_count = if !settled && opening && current > 0 {
        current
    } else {
        expected
    };
    state.reconcile_layer_event(namespace, target_count, &latest)
}

fn run_actions(ipc: &HyprlandIpc, state: &mut RuntimeState, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::SwitchLayout {
                keyboards,
                layout,
                previous,
            } => {
                let mut any_switched = false;
                for keyboard in keyboards {
                    match ipc.switch_layout(&keyboard, layout) {
                        Ok(()) => any_switched = true,
                        Err(err) => {
                            log::warn!("failed to switch {keyboard} to layout {layout}: {err}");
                            state.switch_failed(&keyboard);
                        }
                    }
                }
                if !any_switched {
                    state.set_active_layout(previous);
                }
            }
            Action::QueryKeyboardLayout { keyboard } => {
                match ipc.active_layout_for_keyboard(&keyboard) {
                    Ok(layout) => state.record_keyboard_layout(&keyboard, layout),
                    Err(err) => log::warn!("failed to query active layout for {keyboard}: {err}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use hypr_kblayoutd::config::{Config, KeyboardConfig};
    use hypr_kblayoutd::state::WindowAddr;

    fn test_state() -> RuntimeState {
        test_state_with_layers(&[("rofi", 0)])
    }

    fn test_state_with_layers(layers: &[(&str, u8)]) -> RuntimeState {
        let config = Config {
            keyboards: KeyboardConfig {
                include: vec!["kbd".to_string()],
                exclude_contains: Vec::new(),
            },
            default_layouts: HashMap::new(),
            layer_layouts: layers
                .iter()
                .map(|(namespace, layout)| ((*namespace).to_string(), *layout))
                .collect(),
        };
        let mut state = RuntimeState::new(config, 0);
        state.handle_event(event::Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);
        state
    }

    fn switch(layout: u8, previous: u8) -> Vec<Action> {
        vec![Action::SwitchLayout {
            keyboards: vec!["kbd".to_string()],
            layout,
            previous,
        }]
    }

    fn snapshot(namespaces: &[&str]) -> Vec<String> {
        namespaces
            .iter()
            .map(|namespace| (*namespace).to_string())
            .collect()
    }

    #[test]
    fn production_path_retries_until_open_snapshot_settles() {
        let mut state = test_state();
        let mut snapshots = [snapshot(&[]), snapshot(&["rofi"])].into_iter();
        let mut waits = 0;

        let actions = handle_event_with_layer_snapshot(
            &mut state,
            event::Event::OpenLayer { namespace: "rofi" },
            || Ok::<_, &'static str>(snapshots.next().expect("test snapshot")),
            || waits += 1,
        );

        assert_eq!(actions, switch(0, 1));
        assert_eq!(waits, 1);
        assert_eq!(state.focused_layer_count("rofi"), 1);
    }

    #[test]
    fn settled_close_keeps_a_live_duplicate() {
        let mut state = test_state();
        state.sync_layers(&snapshot(&["rofi", "rofi"]));
        let mut queries = 0;
        let mut waits = 0;

        let actions = handle_event_with_layer_snapshot(
            &mut state,
            event::Event::CloseLayer { namespace: "rofi" },
            || {
                queries += 1;
                Ok::<_, &'static str>(snapshot(&["rofi"]))
            },
            || waits += 1,
        );

        assert_eq!(queries, 1);
        assert_eq!(waits, 0);
        assert!(actions.is_empty());
        assert_eq!(state.focused_layer_count("rofi"), 1);
    }

    #[test]
    fn animated_close_falls_back_to_event_when_snapshot_stays_stale() {
        let mut state = test_state();
        state.sync_layers(&snapshot(&["rofi"]));
        let mut queries = 0;
        let mut waits = 0;

        // An unchanged snapshot is ambiguous: it can be a buffered close or
        // an animating close. After retries, the close event wins.
        let actions = handle_event_with_layer_snapshot(
            &mut state,
            event::Event::CloseLayer { namespace: "rofi" },
            || {
                queries += 1;
                Ok::<_, &'static str>(snapshot(&["rofi"]))
            },
            || waits += 1,
        );

        assert_eq!(queries, LAYER_SNAPSHOT_RETRIES + 1);
        assert_eq!(waits, LAYER_SNAPSHOT_RETRIES);
        assert_eq!(actions, switch(1, 0));
        assert_eq!(state.focused_layer_count("rofi"), 0);
    }

    #[test]
    fn quick_open_does_not_reimport_a_different_fading_layer() {
        let mut state = test_state_with_layers(&[("rofi", 0), ("menu", 2)]);
        state.sync_layers(&snapshot(&["rofi"]));

        assert_eq!(
            handle_event_with_layer_snapshot(
                &mut state,
                event::Event::CloseLayer { namespace: "rofi" },
                || Ok::<_, &'static str>(snapshot(&["rofi"])),
                || {},
            ),
            switch(1, 0)
        );

        assert_eq!(
            handle_event_with_layer_snapshot(
                &mut state,
                event::Event::OpenLayer { namespace: "menu" },
                || Ok::<_, &'static str>(snapshot(&["rofi", "menu"])),
                || {},
            ),
            switch(2, 1)
        );
        assert_eq!(state.focused_layer_count("rofi"), 0);
        assert_eq!(state.focused_layer_count("menu"), 1);
    }

    #[test]
    fn quick_reopen_does_not_count_fading_same_namespace() {
        let mut state = test_state();
        state.sync_layers(&snapshot(&["rofi"]));

        assert_eq!(
            handle_event_with_layer_snapshot(
                &mut state,
                event::Event::CloseLayer { namespace: "rofi" },
                || Ok::<_, &'static str>(snapshot(&["rofi"])),
                || {},
            ),
            switch(1, 0)
        );

        assert_eq!(
            handle_event_with_layer_snapshot(
                &mut state,
                event::Event::OpenLayer { namespace: "rofi" },
                || Ok::<_, &'static str>(snapshot(&["rofi", "rofi"])),
                || {},
            ),
            switch(0, 1)
        );
        assert_eq!(state.focused_layer_count("rofi"), 1);
    }

    #[test]
    fn first_open_falls_back_to_event_when_snapshot_stays_stale() {
        let mut state = test_state();

        let actions = handle_event_with_layer_snapshot(
            &mut state,
            event::Event::OpenLayer { namespace: "rofi" },
            || Ok::<_, &'static str>(snapshot(&[])),
            || {},
        );

        assert_eq!(actions, switch(0, 1));
        assert_eq!(state.focused_layer_count("rofi"), 1);
    }

    #[test]
    fn unsettled_buffered_open_does_not_double_count_snapshot() {
        let mut state = test_state();
        state.sync_layers(&snapshot(&["rofi"]));

        let actions = handle_event_with_layer_snapshot(
            &mut state,
            event::Event::OpenLayer { namespace: "rofi" },
            || Ok::<_, &'static str>(snapshot(&["rofi"])),
            || {},
        );

        assert!(actions.is_empty());
        assert_eq!(state.focused_layer_count("rofi"), 1);
    }

    #[test]
    fn snapshot_error_uses_the_shared_delta_fallback() {
        let mut state = test_state();

        let actions = handle_event_with_layer_snapshot(
            &mut state,
            event::Event::OpenLayer { namespace: "rofi" },
            || Err::<Vec<String>, _>("test query failure"),
            || {},
        );

        assert_eq!(actions, switch(0, 1));
        assert_eq!(state.focused_layer_count("rofi"), 1);
    }

    #[test]
    fn untracked_event_skips_snapshot_when_no_layer_is_focused() {
        let mut state = test_state();
        let mut queried = false;

        let actions = handle_event_with_layer_snapshot(
            &mut state,
            event::Event::OpenLayer {
                namespace: "waybar",
            },
            || {
                queried = true;
                Ok::<_, &'static str>(snapshot(&["waybar"]))
            },
            || {},
        );

        assert!(actions.is_empty());
        assert!(!queried);
    }

    #[test]
    fn untracked_event_still_reconciles_stranded_layer_focus() {
        let mut state = test_state();
        state.sync_layers(&snapshot(&["rofi"]));
        let mut queried = false;

        let actions = handle_event_with_layer_snapshot(
            &mut state,
            event::Event::OpenLayer {
                namespace: "waybar",
            },
            || {
                queried = true;
                Ok::<_, &'static str>(snapshot(&["waybar"]))
            },
            || {},
        );

        assert!(queried);
        assert_eq!(actions, switch(1, 0));
        assert_eq!(state.focused_layer_count("rofi"), 0);
    }
}
