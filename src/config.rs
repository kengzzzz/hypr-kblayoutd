use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::state::LayoutIndex;

const DEFAULT_EXCLUDES: &[&str] = &["wlr_virtual_keyboard_v", "yubikey"];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub keyboards: KeyboardConfig,
    pub default_layouts: HashMap<String, LayoutIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardConfig {
    pub include: Vec<String>,
    pub exclude_contains: Vec<String>,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    InvalidLayoutIndex { class_name: String, value: u16 },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read config: {err}"),
            Self::Parse(err) => write!(f, "failed to parse config: {err}"),
            Self::InvalidLayoutIndex { class_name, value } => {
                write!(
                    f,
                    "layout index {value} for class {class_name:?} is too large"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Default for KeyboardConfig {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude_contains: DEFAULT_EXCLUDES.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    keyboards: Option<RawKeyboardConfig>,
    default_layouts: Option<HashMap<String, u16>>,
}

#[derive(Debug, Default, Deserialize)]
struct RawKeyboardConfig {
    #[serde(default)]
    include: Vec<String>,
    exclude_contains: Option<Vec<String>>,
}

pub fn default_config_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("hypr-kblayoutd").join("config.toml"))
}

pub fn load_default() -> Result<Config, ConfigError> {
    match default_config_path() {
        Some(path) => load_optional(path),
        None => Ok(Config::default()),
    }
}

pub fn load_optional(path: impl AsRef<Path>) -> Result<Config, ConfigError> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(Config::default());
    }
    parse(&fs::read_to_string(path).map_err(ConfigError::Io)?)
}

pub fn parse(input: &str) -> Result<Config, ConfigError> {
    let raw: RawConfig = toml::from_str(input).map_err(ConfigError::Parse)?;
    let raw_keyboards = raw.keyboards.unwrap_or_default();
    let mut default_layouts = HashMap::new();

    for (class_name, value) in raw.default_layouts.unwrap_or_default() {
        let index = LayoutIndex::try_from(value).map_err(|_| ConfigError::InvalidLayoutIndex {
            class_name: class_name.clone(),
            value,
        })?;
        default_layouts.insert(class_name, index);
    }

    Ok(Config {
        keyboards: KeyboardConfig {
            include: raw_keyboards.include,
            exclude_contains: raw_keyboards
                .exclude_contains
                .unwrap_or_else(|| DEFAULT_EXCLUDES.iter().map(|s| (*s).to_string()).collect()),
        },
        default_layouts,
    })
}

impl KeyboardConfig {
    pub fn is_excluded(&self, keyboard: &str) -> bool {
        self.exclude_contains
            .iter()
            .any(|fragment| !fragment.is_empty() && keyboard.contains(fragment))
    }

    pub fn is_configured(&self) -> bool {
        !self.include.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_uses_defaults() {
        let cfg = load_optional("/tmp/path-that-should-not-exist-hpwl.toml").unwrap();
        assert!(cfg.keyboards.include.is_empty());
        assert!(cfg.keyboards.is_excluded("wlr_virtual_keyboard_v1"));
        assert!(cfg.keyboards.is_excluded("my-yubikey"));
        assert!(cfg.default_layouts.is_empty());
    }

    #[test]
    fn parses_v2_config() {
        let cfg = parse(
            r#"
                [keyboards]
                include = ["keychron-keychron-k2"]
                exclude_contains = ["virtual"]

                [default_layouts]
                "org.telegram.desktop" = 1
                "firefox" = 0
            "#,
        )
        .unwrap();

        assert_eq!(cfg.keyboards.include, ["keychron-keychron-k2"]);
        assert!(cfg.keyboards.is_excluded("my-virtual-keyboard"));
        assert_eq!(cfg.default_layouts["org.telegram.desktop"], 1);
        assert_eq!(cfg.default_layouts["firefox"], 0);
    }

    #[test]
    fn default_excludes_are_used_when_keyboard_section_is_empty() {
        let cfg = parse(
            r#"
                [default_layouts]
                "discord" = 1
            "#,
        )
        .unwrap();

        assert!(cfg.keyboards.is_excluded("wlr_virtual_keyboard_v2"));
        assert_eq!(cfg.default_layouts["discord"], 1);
    }
}
