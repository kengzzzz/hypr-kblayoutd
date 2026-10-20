use serde::de::IgnoredAny;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::state::LayoutIndex;

const DEFAULT_EXCLUDES: &[&str] = &[
    "wlr_virtual_keyboard_v",
    "yubikey",
    "power-button",
    "sleep-button",
    "video-bus",
    "consumer-control",
    "system-control",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub keyboards: KeyboardConfig,
    pub default_layouts: HashMap<String, LayoutIndex>,
    pub layer_layouts: HashMap<String, LayoutIndex>,
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
    InvalidLayoutIndex { key: String, value: u16 },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read config: {err}"),
            Self::Parse(err) => write!(f, "failed to parse config: {err}"),
            Self::InvalidLayoutIndex { key, value } => {
                write!(f, "layout index {value} for {key:?} is too large")
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

// Unknown keys are warned about, not rejected: an error would restart-loop under systemd.
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    keyboards: Option<RawKeyboardConfig>,
    default_layouts: Option<HashMap<String, u16>>,
    layer_layouts: Option<HashMap<String, u16>>,
    #[serde(flatten)]
    unknown: BTreeMap<String, IgnoredAny>,
}

#[derive(Debug, Default, Deserialize)]
struct RawKeyboardConfig {
    #[serde(default)]
    include: Vec<String>,
    exclude_contains: Option<Vec<String>>,
    #[serde(flatten)]
    unknown: BTreeMap<String, IgnoredAny>,
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
    for key in unknown_keys(&raw) {
        log::warn!("ignoring unknown config key {key:?}");
    }
    let raw_keyboards = raw.keyboards.unwrap_or_default();
    let default_layouts = layout_map(raw.default_layouts.unwrap_or_default())?;
    let layer_layouts = layout_map(raw.layer_layouts.unwrap_or_default())?;

    Ok(Config {
        keyboards: KeyboardConfig {
            include: raw_keyboards.include,
            exclude_contains: raw_keyboards
                .exclude_contains
                .unwrap_or_else(|| DEFAULT_EXCLUDES.iter().map(|s| (*s).to_string()).collect()),
        },
        default_layouts,
        layer_layouts,
    })
}

fn unknown_keys(raw: &RawConfig) -> Vec<String> {
    let top = raw.unknown.keys().cloned();
    let keyboards = raw
        .keyboards
        .iter()
        .flat_map(|keyboards| keyboards.unknown.keys())
        .map(|key| format!("keyboards.{key}"));
    top.chain(keyboards).collect()
}

fn layout_map(raw: HashMap<String, u16>) -> Result<HashMap<String, LayoutIndex>, ConfigError> {
    raw.into_iter()
        .map(|(key, value)| {
            let index =
                LayoutIndex::try_from(value).map_err(|_| ConfigError::InvalidLayoutIndex {
                    key: key.clone(),
                    value,
                })?;
            Ok((key, index))
        })
        .collect()
}

impl Config {
    pub fn layouts_out_of_range(
        &self,
        layout_count: usize,
    ) -> Vec<(&'static str, &str, LayoutIndex)> {
        let sections = [
            ("default_layouts", &self.default_layouts),
            ("layer_layouts", &self.layer_layouts),
        ];
        let mut out: Vec<_> = sections
            .into_iter()
            .flat_map(|(section, layouts)| {
                layouts
                    .iter()
                    .filter(|(_, layout)| usize::from(**layout) >= layout_count)
                    .map(move |(key, layout)| (section, key.as_str(), *layout))
            })
            .collect();
        out.sort();
        out
    }
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

                [layer_layouts]
                "rofi" = 0
            "#,
        )
        .unwrap();

        assert_eq!(cfg.keyboards.include, ["keychron-keychron-k2"]);
        assert!(cfg.keyboards.is_excluded("my-virtual-keyboard"));
        assert_eq!(cfg.default_layouts["org.telegram.desktop"], 1);
        assert_eq!(cfg.default_layouts["firefox"], 0);
        assert_eq!(cfg.layer_layouts["rofi"], 0);
    }

    #[test]
    fn layer_layouts_are_empty_by_default() {
        let cfg = parse(
            r#"
                [default_layouts]
                "discord" = 1
            "#,
        )
        .unwrap();

        assert!(cfg.layer_layouts.is_empty());
    }

    #[test]
    fn default_excludes_skip_non_typing_devices() {
        let cfg = Config::default();

        for device in [
            "power-button",
            "power-button-1",
            "video-bus",
            "c-media-electronics-inc.-usb-advanced-audio-device-consumer-control",
            "logitech-usb-receiver-system-control",
        ] {
            assert!(cfg.keyboards.is_excluded(device), "{device}");
        }
        assert!(!cfg.keyboards.is_excluded("keychron-keychron-k2"));
    }

    #[test]
    fn unknown_keys_are_reported_but_not_rejected() {
        let input = r#"
            [keyboard]
            include = ["kbd"]

            [keyboards]
            exclude = ["virtual"]

            [layer_layouts]
            "rofi" = 0
        "#;
        let raw: RawConfig = toml::from_str(input).unwrap();

        assert_eq!(unknown_keys(&raw), ["keyboard", "keyboards.exclude"]);
        assert_eq!(parse(input).unwrap().layer_layouts["rofi"], 0);
    }

    #[test]
    fn reports_layouts_beyond_configured_count() {
        let cfg = parse(
            r#"
                [default_layouts]
                "firefox" = 0
                "discord" = 2

                [layer_layouts]
                "rofi" = 1
                "wofi" = 3
            "#,
        )
        .unwrap();

        assert_eq!(
            cfg.layouts_out_of_range(2),
            [
                ("default_layouts", "discord", 2),
                ("layer_layouts", "wofi", 3)
            ]
        );
        assert!(cfg.layouts_out_of_range(4).is_empty());
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
