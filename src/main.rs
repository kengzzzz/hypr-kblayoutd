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
    let _instance = SingleInstance::acquire("hypr-kblayoutd")?;
    let config = config::load_default()?;
    let paths = HyprlandPaths::discover()?;
    let ipc = HyprlandIpc::new(paths);

    let layout_count = ipc.configured_layout_count()?;
    if layout_count < 2 && !ipc.kb_file_is_set()? {
        return Err("Hyprland needs at least two configured keyboard layouts".into());
    }

    let initial_layout = ipc.initial_active_layout()?;
    let mut state = RuntimeState::new(config, initial_layout);
    listen_forever(&ipc, &mut state)
}

fn listen_forever(
    ipc: &HyprlandIpc,
    state: &mut RuntimeState,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut backoff = Duration::from_millis(100);

    loop {
        match listen_once(ipc, state) {
            Ok(()) => log::warn!("Hyprland event socket closed; reconnecting"),
            Err(err) => log::warn!("Hyprland event socket error: {err}; reconnecting"),
        }

        thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(5));
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
            Action::SwitchLayout { keyboards, layout } => {
                for keyboard in keyboards {
                    if let Err(err) = ipc.switch_layout(&keyboard, layout) {
                        log::warn!("failed to switch {keyboard} to layout {layout}: {err}");
                    }
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
