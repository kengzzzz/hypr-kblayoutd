use std::io::BufRead;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;

use hypr_kblayoutd::config;
use hypr_kblayoutd::event;
use hypr_kblayoutd::ipc::{HyprlandIpc, HyprlandPaths};
use hypr_kblayoutd::single_instance::SingleInstance;
use hypr_kblayoutd::state::{Action, RuntimeState};

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

    let initial_layout = ipc.current_active_layout()?;
    let mut state = RuntimeState::new(config, initial_layout);
    listen_forever(&ipc, &mut state)
}

fn listen_forever(
    ipc: &HyprlandIpc,
    state: &mut RuntimeState,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut backoff = Duration::from_millis(100);

    loop {
        resync_state(ipc, state);

        match listen_once(ipc, state) {
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
        Ok((layout, active, clients))
    });

    match queried {
        Ok((layout, active, clients)) => {
            let actions = state.resync(active, layout, &clients);
            run_actions(ipc, state, actions);
        }
        Err(err) => log::warn!("state resync skipped: {err}"),
    }
}

fn listen_once(ipc: &HyprlandIpc, state: &mut RuntimeState) -> std::io::Result<()> {
    let stream = UnixStream::connect(&ipc.paths().event_socket)?;
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
                let actions = state.handle_event(parsed);
                run_actions(ipc, state, actions);
            }
            Err(err) => log::debug!("ignored malformed Hyprland event {line:?}: {err:?}"),
        }
    }
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
