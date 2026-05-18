use serde::Deserialize;
use std::env;
use std::fmt;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use crate::event::parse_layout_index;
use crate::state::LayoutIndex;

#[derive(Debug)]
pub enum IpcError {
    MissingHyprlandSignature,
    Io(std::io::Error),
    Json(serde_json::Error),
    MissingKeyboard(String),
    InvalidLayoutIndex(u64),
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHyprlandSignature => write!(f, "HYPRLAND_INSTANCE_SIGNATURE is not set"),
            Self::Io(err) => write!(f, "Hyprland IPC error: {err}"),
            Self::Json(err) => write!(f, "failed to parse Hyprland JSON: {err}"),
            Self::MissingKeyboard(keyboard) => write!(f, "keyboard {keyboard:?} was not found"),
            Self::InvalidLayoutIndex(index) => {
                write!(
                    f,
                    "Hyprland returned layout index {index}, which is too large"
                )
            }
        }
    }
}

impl std::error::Error for IpcError {}

impl From<std::io::Error> for IpcError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for IpcError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[derive(Debug, Clone)]
pub struct HyprlandPaths {
    pub command_socket: PathBuf,
    pub event_socket: PathBuf,
}

#[derive(Debug)]
pub struct HyprlandIpc {
    paths: HyprlandPaths,
}

#[derive(Debug, Deserialize)]
struct OptionResponse {
    str: String,
    #[serde(default)]
    set: bool,
}

#[derive(Debug, Deserialize)]
struct DevicesResponse {
    keyboards: Vec<KeyboardResponse>,
}

#[derive(Debug, Deserialize)]
struct KeyboardResponse {
    name: String,
    active_layout_index: u64,
    #[serde(default)]
    main: bool,
}

impl HyprlandPaths {
    pub fn discover() -> Result<Self, IpcError> {
        let signature = env::var("HYPRLAND_INSTANCE_SIGNATURE")
            .map_err(|_| IpcError::MissingHyprlandSignature)?;
        let runtime_path = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .map(|runtime| runtime.join("hypr").join(&signature));
        let fallback = PathBuf::from("/tmp").join("hypr").join(&signature);
        let base = runtime_path
            .filter(|path| path.join(".socket.sock").exists())
            .unwrap_or(fallback);

        Ok(Self {
            command_socket: base.join(".socket.sock"),
            event_socket: base.join(".socket2.sock"),
        })
    }
}

impl HyprlandIpc {
    pub fn new(paths: HyprlandPaths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &HyprlandPaths {
        &self.paths
    }

    pub fn command(&self, command: &str) -> Result<String, IpcError> {
        let mut stream = UnixStream::connect(&self.paths.command_socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(command.as_bytes())?;
        let _ = stream.shutdown(Shutdown::Write);

        let mut output = String::new();
        stream.read_to_string(&mut output)?;
        Ok(output)
    }

    pub fn json_command<T: for<'de> Deserialize<'de>>(&self, command: &str) -> Result<T, IpcError> {
        Ok(serde_json::from_str(&self.command(command)?)?)
    }

    pub fn switch_layout(&self, keyboard: &str, layout: LayoutIndex) -> Result<(), IpcError> {
        let output = self.command(&switch_layout_command(keyboard, layout))?;
        if output.trim() == "ok" || output.trim().is_empty() {
            Ok(())
        } else {
            log::debug!("switchxkblayout response for {keyboard}: {}", output.trim());
            Ok(())
        }
    }

    pub fn configured_layout_count(&self) -> Result<usize, IpcError> {
        let response: OptionResponse = self.json_command("j/getoption input:kb_layout")?;
        if response.str.is_empty() || response.str == "[[EMPTY]]" {
            Ok(0)
        } else {
            Ok(response
                .str
                .split(',')
                .filter(|layout| !layout.trim().is_empty())
                .count())
        }
    }

    pub fn kb_file_is_set(&self) -> Result<bool, IpcError> {
        let response: OptionResponse = self.json_command("j/getoption input:kb_file")?;
        Ok(response.set || response.str != "[[EMPTY]]")
    }

    pub fn active_layout_for_keyboard(&self, keyboard_name: &str) -> Result<LayoutIndex, IpcError> {
        let devices: DevicesResponse = self.json_command("j/devices")?;
        let Some(keyboard) = devices
            .keyboards
            .into_iter()
            .find(|keyboard| keyboard.name == keyboard_name)
        else {
            return Err(IpcError::MissingKeyboard(keyboard_name.to_string()));
        };

        parse_layout_index(keyboard.active_layout_index)
            .ok_or(IpcError::InvalidLayoutIndex(keyboard.active_layout_index))
    }

    pub fn initial_active_layout(&self) -> Result<LayoutIndex, IpcError> {
        let devices: DevicesResponse = self.json_command("j/devices")?;
        let keyboard = devices
            .keyboards
            .iter()
            .find(|keyboard| keyboard.main)
            .or_else(|| devices.keyboards.first())
            .ok_or_else(|| IpcError::MissingKeyboard("<any>".to_string()))?;

        parse_layout_index(keyboard.active_layout_index)
            .ok_or(IpcError::InvalidLayoutIndex(keyboard.active_layout_index))
    }
}

pub fn switch_layout_command(keyboard: &str, layout: LayoutIndex) -> String {
    format!("switchxkblayout {keyboard} {layout}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_layout_command_uses_raw_ipc_format_without_separator() {
        assert_eq!(
            switch_layout_command("keychron-keychron-k2", 1),
            "switchxkblayout keychron-keychron-k2 1"
        );
    }
}
