use std::collections::HashMap;
use std::fmt;

use crate::config::Config;
use crate::event::Event;

pub type LayoutIndex = u8;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct WindowAddr(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    SwitchLayout {
        keyboards: Vec<String>,
        layout: LayoutIndex,
        previous: LayoutIndex,
    },
    QueryKeyboardLayout {
        keyboard: String,
    },
}

#[derive(Debug, Clone)]
pub struct RuntimeState {
    windows: HashMap<WindowAddr, LayoutIndex>,
    active_window: Option<WindowAddr>,
    active_class: Option<String>,
    active_layout: LayoutIndex,
    keyboards: Vec<String>,
    configured_keyboards: bool,
    exclude_contains: Vec<String>,
    defaults_by_class: HashMap<String, LayoutIndex>,
    pending_echoes: HashMap<String, u8>,
}

impl RuntimeState {
    pub fn new(config: Config, active_layout: LayoutIndex) -> Self {
        let configured_keyboards = !config.keyboards.include.is_empty();
        Self {
            windows: HashMap::new(),
            active_window: None,
            active_class: None,
            active_layout,
            keyboards: config.keyboards.include,
            configured_keyboards,
            exclude_contains: config.keyboards.exclude_contains,
            defaults_by_class: config.default_layouts,
            pending_echoes: HashMap::new(),
        }
    }

    pub fn handle_event(&mut self, event: Event<'_>) -> Vec<Action> {
        match event {
            Event::ActiveWindow { class_name } => {
                self.active_class = Some(class_name.to_string());
                Vec::new()
            }
            Event::ActiveWindowV2 { addr } => self.activate_window(addr),
            Event::EmptyActiveWindow => {
                self.active_window = None;
                self.active_class = None;
                Vec::new()
            }
            Event::CloseWindow { addr } => {
                self.windows.remove(&addr);
                Vec::new()
            }
            Event::ActiveLayout { keyboard, .. } => self.handle_active_layout(keyboard),
            Event::Ignored => Vec::new(),
        }
    }

    pub fn record_keyboard_layout(&mut self, keyboard: &str, layout: LayoutIndex) {
        if !self.is_managed_keyboard(keyboard) {
            log::debug!("keyboard skipped while recording layout: keyboard={keyboard}");
            return;
        }
        self.active_layout = layout;
        if let Some(addr) = self.active_window {
            self.windows.insert(addr, layout);
            log::debug!(
                "manual layout change recorded: keyboard={keyboard} window={addr} layout={layout}"
            );
        } else {
            log::debug!(
                "manual layout change recorded without active window: keyboard={keyboard} layout={layout}"
            );
        }
    }

    pub fn set_active_layout(&mut self, layout: LayoutIndex) {
        self.active_layout = layout;
    }

    pub fn active_layout(&self) -> LayoutIndex {
        self.active_layout
    }

    pub fn active_window_layout(&self) -> Option<LayoutIndex> {
        self.active_window
            .and_then(|addr| self.windows.get(&addr).copied())
    }

    pub fn keyboard_names(&self) -> &[String] {
        &self.keyboards
    }

    /// Re-align state with Hyprland after events may have been missed
    /// (startup, or a gap while the event socket was down).
    pub fn resync(
        &mut self,
        active: Option<(WindowAddr, String)>,
        actual_layout: LayoutIndex,
        live_windows: &[WindowAddr],
    ) -> Vec<Action> {
        self.windows.retain(|addr, _| live_windows.contains(addr));
        self.pending_echoes.clear();
        self.active_layout = actual_layout;

        let Some((addr, class_name)) = active else {
            self.active_window = None;
            self.active_class = None;
            return Vec::new();
        };

        self.active_window = Some(addr);
        self.active_class = Some(class_name);
        match self.windows.get(&addr).copied() {
            Some(remembered) => {
                log::debug!(
                    "resync: restoring known window: address={addr} target_layout={remembered} actual_layout={actual_layout}"
                );
                self.switch_actions(remembered)
            }
            None => {
                // Window appeared while we were disconnected; adopt whatever
                // the user is currently typing with instead of forcing a default.
                self.windows.insert(addr, actual_layout);
                log::debug!("resync: learned window: address={addr} layout={actual_layout}");
                Vec::new()
            }
        }
    }

    fn activate_window(&mut self, addr: WindowAddr) -> Vec<Action> {
        self.active_window = Some(addr);
        let previous = self.active_layout;
        let target = match self.windows.get(&addr).copied() {
            Some(layout) => {
                log::debug!(
                    "known window restored: address={addr} target_layout={layout} previous_active_layout={previous}"
                );
                layout
            }
            None => {
                let layout = self.default_layout_for_active_class();
                self.windows.insert(addr, layout);
                log::debug!(
                    "new window seen: address={addr} class={} chosen_layout={layout}",
                    self.active_class.as_deref().unwrap_or("<unknown>")
                );
                layout
            }
        };

        self.switch_actions(target)
    }

    fn handle_active_layout(&mut self, keyboard: &str) -> Vec<Action> {
        if self.is_excluded_keyboard(keyboard) {
            log::debug!("keyboard skipped due to exclude rule: keyboard={keyboard}");
            return Vec::new();
        }
        if self.consume_pending_echo(keyboard) {
            log::debug!("own layout switch echoed back, ignored: keyboard={keyboard}");
            return Vec::new();
        }
        if self.configured_keyboards {
            if !self.keyboards.iter().any(|known| known == keyboard) {
                log::debug!("keyboard skipped due to include rule: keyboard={keyboard}");
                return Vec::new();
            }
        } else if !self.keyboards.iter().any(|known| known == keyboard) {
            self.keyboards.push(keyboard.to_string());
            log::debug!("keyboard learned: keyboard={keyboard}");
        }

        vec![Action::QueryKeyboardLayout {
            keyboard: keyboard.to_string(),
        }]
    }

    fn switch_actions(&mut self, target: LayoutIndex) -> Vec<Action> {
        if target == self.active_layout {
            log::debug!("layout switch skipped: target_layout={target} already active");
            return Vec::new();
        }
        if self.keyboards.is_empty() {
            log::debug!(
                "layout switch skipped: target_layout={target} no managed keyboards are known"
            );
            self.active_layout = target;
            return Vec::new();
        }
        let previous = self.active_layout;
        self.active_layout = target;
        for keyboard in &self.keyboards {
            *self.pending_echoes.entry(keyboard.clone()).or_insert(0) += 1;
        }
        vec![Action::SwitchLayout {
            keyboards: self.keyboards.clone(),
            layout: target,
            previous,
        }]
    }

    pub fn switch_failed(&mut self, keyboard: &str) {
        self.consume_pending_echo(keyboard);
    }

    fn consume_pending_echo(&mut self, keyboard: &str) -> bool {
        let Some(count) = self.pending_echoes.get_mut(keyboard) else {
            return false;
        };
        *count -= 1;
        if *count == 0 {
            self.pending_echoes.remove(keyboard);
        }
        true
    }

    fn default_layout_for_active_class(&self) -> LayoutIndex {
        let Some(class_name) = &self.active_class else {
            return 0;
        };
        self.defaults_by_class.get(class_name).copied().unwrap_or(0)
    }

    fn is_managed_keyboard(&self, keyboard: &str) -> bool {
        !self.is_excluded_keyboard(keyboard)
            && (!self.configured_keyboards || self.keyboards.iter().any(|known| known == keyboard))
    }

    fn is_excluded_keyboard(&self, keyboard: &str) -> bool {
        self.exclude_contains
            .iter()
            .any(|fragment| !fragment.is_empty() && keyboard.contains(fragment))
    }
}

impl fmt::Display for WindowAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, KeyboardConfig};

    fn config(include: &[&str], defaults: &[(&str, LayoutIndex)]) -> Config {
        Config {
            keyboards: KeyboardConfig {
                include: include.iter().map(|s| (*s).to_string()).collect(),
                exclude_contains: vec!["virtual".to_string(), "yubikey".to_string()],
            },
            default_layouts: defaults
                .iter()
                .map(|(class, layout)| ((*class).to_string(), *layout))
                .collect(),
        }
    }

    #[test]
    fn new_window_uses_class_default() {
        let mut state = RuntimeState::new(config(&["kbd"], &[("firefox", 1)]), 0);
        state.handle_event(Event::ActiveWindow {
            class_name: "firefox",
        });

        assert_eq!(
            state.handle_event(Event::ActiveWindowV2 {
                addr: WindowAddr(1)
            }),
            vec![Action::SwitchLayout {
                keyboards: vec!["kbd".to_string()],
                layout: 1,
                previous: 0
            }]
        );
        assert_eq!(state.active_window_layout(), Some(1));
    }

    #[test]
    fn known_window_restores_saved_layout() {
        let mut state = RuntimeState::new(config(&["kbd"], &[]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(2),
        });

        assert_eq!(
            state.handle_event(Event::ActiveWindowV2 {
                addr: WindowAddr(1)
            }),
            vec![Action::SwitchLayout {
                keyboards: vec!["kbd".to_string()],
                layout: 1,
                previous: 0
            }]
        );
    }

    #[test]
    fn same_layout_does_not_emit_switch() {
        let mut state = RuntimeState::new(config(&["kbd"], &[("firefox", 0)]), 0);
        state.handle_event(Event::ActiveWindow {
            class_name: "firefox",
        });

        assert!(state
            .handle_event(Event::ActiveWindowV2 {
                addr: WindowAddr(1)
            })
            .is_empty());
    }

    #[test]
    fn active_layout_learns_keyboard_when_unconfigured() {
        let mut state = RuntimeState::new(config(&[], &[]), 0);
        assert_eq!(
            state.handle_event(Event::ActiveLayout {
                keyboard: "kbd",
                layout_name: "English"
            }),
            vec![Action::QueryKeyboardLayout {
                keyboard: "kbd".to_string()
            }]
        );
        assert_eq!(state.keyboard_names(), ["kbd"]);
    }

    #[test]
    fn configured_keyboard_ignores_other_keyboards() {
        let mut state = RuntimeState::new(config(&["kbd"], &[]), 0);
        assert!(state
            .handle_event(Event::ActiveLayout {
                keyboard: "other",
                layout_name: "English"
            })
            .is_empty());
    }

    #[test]
    fn excluded_keyboard_is_ignored() {
        let mut state = RuntimeState::new(config(&[], &[]), 0);
        assert!(state
            .handle_event(Event::ActiveLayout {
                keyboard: "virtual-keyboard",
                layout_name: "English"
            })
            .is_empty());
        assert!(state.keyboard_names().is_empty());
    }

    #[test]
    fn close_window_forgets_saved_layout() {
        let mut state = RuntimeState::new(config(&["kbd"], &[]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);
        state.handle_event(Event::CloseWindow {
            addr: WindowAddr(1),
        });
        state.set_active_layout(1);

        assert_eq!(
            state.handle_event(Event::ActiveWindowV2 {
                addr: WindowAddr(1)
            }),
            vec![Action::SwitchLayout {
                keyboards: vec!["kbd".to_string()],
                layout: 0,
                previous: 1
            }]
        );
    }

    #[test]
    fn empty_active_window_clears_active_window() {
        let mut state = RuntimeState::new(config(&["kbd"], &[]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.handle_event(Event::CloseWindow {
            addr: WindowAddr(1),
        });
        state.handle_event(Event::EmptyActiveWindow);

        // A manual layout change on an empty workspace must not resurrect
        // the closed window's entry.
        state.record_keyboard_layout("kbd", 1);

        assert_eq!(
            state.handle_event(Event::ActiveWindowV2 {
                addr: WindowAddr(1)
            }),
            vec![Action::SwitchLayout {
                keyboards: vec!["kbd".to_string()],
                layout: 0,
                previous: 1
            }]
        );
    }

    #[test]
    fn own_switch_echo_is_ignored_once() {
        let mut state = RuntimeState::new(config(&["kbd"], &[("firefox", 1)]), 0);
        state.handle_event(Event::ActiveWindow {
            class_name: "firefox",
        });
        assert!(!state
            .handle_event(Event::ActiveWindowV2 {
                addr: WindowAddr(1)
            })
            .is_empty());

        let echo = Event::ActiveLayout {
            keyboard: "kbd",
            layout_name: "Thai",
        };
        assert!(state.handle_event(echo.clone()).is_empty());
        assert_eq!(
            state.handle_event(echo),
            vec![Action::QueryKeyboardLayout {
                keyboard: "kbd".to_string()
            }]
        );
    }

    #[test]
    fn switch_failed_clears_pending_echo() {
        let mut state = RuntimeState::new(config(&["kbd"], &[("firefox", 1)]), 0);
        state.handle_event(Event::ActiveWindow {
            class_name: "firefox",
        });
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.switch_failed("kbd");

        assert_eq!(
            state.handle_event(Event::ActiveLayout {
                keyboard: "kbd",
                layout_name: "English"
            }),
            vec![Action::QueryKeyboardLayout {
                keyboard: "kbd".to_string()
            }]
        );
    }

    #[test]
    fn resync_prunes_closed_windows() {
        let mut state = RuntimeState::new(config(&["kbd"], &[]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);

        assert!(state.resync(None, 0, &[]).is_empty());

        // Window 1 is gone; re-seeing its address treats it as new.
        assert!(state
            .handle_event(Event::ActiveWindowV2 {
                addr: WindowAddr(1)
            })
            .is_empty());
        assert_eq!(state.active_window_layout(), Some(0));
    }

    #[test]
    fn resync_restores_known_active_window() {
        let mut state = RuntimeState::new(config(&["kbd"], &[]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);

        assert_eq!(
            state.resync(
                Some((WindowAddr(1), "firefox".to_string())),
                0,
                &[WindowAddr(1)]
            ),
            vec![Action::SwitchLayout {
                keyboards: vec!["kbd".to_string()],
                layout: 1,
                previous: 0
            }]
        );
    }

    #[test]
    fn resync_learns_unknown_active_window_without_switching() {
        let mut state = RuntimeState::new(config(&["kbd"], &[("firefox", 0)]), 0);

        assert!(state
            .resync(
                Some((WindowAddr(7), "firefox".to_string())),
                1,
                &[WindowAddr(7)]
            )
            .is_empty());
        assert_eq!(state.active_window_layout(), Some(1));
        assert_eq!(state.active_layout(), 1);
    }

    #[test]
    fn resync_clears_pending_echoes() {
        let mut state = RuntimeState::new(config(&["kbd"], &[("firefox", 1)]), 0);
        state.handle_event(Event::ActiveWindow {
            class_name: "firefox",
        });
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });

        state.resync(
            Some((WindowAddr(1), "firefox".to_string())),
            1,
            &[WindowAddr(1)],
        );

        assert_eq!(
            state.handle_event(Event::ActiveLayout {
                keyboard: "kbd",
                layout_name: "English"
            }),
            vec![Action::QueryKeyboardLayout {
                keyboard: "kbd".to_string()
            }]
        );
    }
}
