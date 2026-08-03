use std::collections::HashMap;
use std::fmt;

use regex_lite::Regex;

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
    default_patterns: Vec<(Regex, LayoutIndex)>,
    // Keyed by namespace, not address: a launcher gets a fresh surface on
    // every open but should keep its layout across them.
    layers: HashMap<String, LayoutIndex>,
    // Tracked namespaces currently holding keyboard focus, innermost last.
    focused_layers: Vec<String>,
    layer_defaults: HashMap<String, LayoutIndex>,
    layer_patterns: Vec<(Regex, LayoutIndex)>,
    pending_echoes: HashMap<String, u8>,
}

impl RuntimeState {
    pub fn new(config: Config, active_layout: LayoutIndex) -> Self {
        let configured_keyboards = !config.keyboards.include.is_empty();
        let default_patterns = compile_patterns(&config.default_layouts, "default_layouts");
        let layer_patterns = compile_patterns(&config.layer_layouts, "layer_layouts");
        Self {
            windows: HashMap::new(),
            active_window: None,
            active_class: None,
            active_layout,
            keyboards: config.keyboards.include,
            configured_keyboards,
            exclude_contains: config.keyboards.exclude_contains,
            defaults_by_class: config.default_layouts,
            default_patterns,
            layers: HashMap::new(),
            focused_layers: Vec::new(),
            layer_defaults: config.layer_layouts,
            layer_patterns,
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
            Event::OpenLayer { namespace } => self.apply_layer_event(namespace, true),
            Event::CloseLayer { namespace } => self.apply_layer_event(namespace, false),
            Event::Ignored => Vec::new(),
        }
    }

    pub fn record_keyboard_layout(&mut self, keyboard: &str, layout: LayoutIndex) {
        if !self.is_managed_keyboard(keyboard) {
            log::debug!("keyboard skipped while recording layout: keyboard={keyboard}");
            return;
        }
        self.active_layout = layout;
        if let Some(namespace) = self.focused_layers.last().cloned() {
            self.layers.insert(namespace.clone(), layout);
            log::debug!(
                "manual layout change recorded: keyboard={keyboard} layer={namespace} layout={layout}"
            );
        } else if let Some(addr) = self.active_window {
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

    pub fn focused_layer_count(&self, namespace: &str) -> usize {
        self.focused_layers
            .iter()
            .filter(|known| known.as_str() == namespace)
            .count()
    }

    pub fn should_reconcile_layer_event(&self, namespace: &str) -> bool {
        !self.focused_layers.is_empty() || self.layer_default(namespace).is_some()
    }

    pub fn seed_keyboard(&mut self, keyboard: &str) {
        if self.configured_keyboards
            || self.is_excluded_keyboard(keyboard)
            || self.keyboards.iter().any(|known| known == keyboard)
        {
            return;
        }
        self.keyboards.push(keyboard.to_string());
        log::debug!("keyboard seeded: keyboard={keyboard}");
    }

    /// Re-align state with Hyprland after events may have been missed
    /// (startup, or a gap while the event socket was down).
    pub fn resync(
        &mut self,
        active: Option<(WindowAddr, String)>,
        actual_layout: LayoutIndex,
        live_windows: &[WindowAddr],
        live_layers: &[String],
    ) -> Vec<Action> {
        self.windows.retain(|addr, _| live_windows.contains(addr));
        self.pending_echoes.clear();
        self.active_layout = actual_layout;
        let layer_target = self.reconcile_layers(live_layers, Some(actual_layout));

        let window_target = match active {
            Some((addr, class_name)) => {
                self.active_window = Some(addr);
                self.active_class = Some(class_name);
                match self.windows.get(&addr).copied() {
                    Some(remembered) => Some(remembered),
                    None => {
                        // The active layout belongs to the layer when one is
                        // mapped, so do not learn it for the window underneath.
                        let layout = if layer_target.is_some() {
                            self.default_layout_for_active_class()
                        } else {
                            actual_layout
                        };
                        self.windows.insert(addr, layout);
                        log::debug!("resync: learned window: address={addr} layout={layout}");
                        None
                    }
                }
            }
            None => {
                self.active_window = None;
                self.active_class = None;
                None
            }
        };

        if let Some(target) = layer_target {
            log::debug!(
                "resync: restoring mapped layer: target_layout={target} actual_layout={actual_layout}"
            );
            return self.switch_actions(target);
        }

        let Some(target) = window_target else {
            return Vec::new();
        };
        log::debug!(
            "resync: restoring known window: target_layout={target} actual_layout={actual_layout}"
        );
        self.switch_actions(target)
    }

    fn activate_window(&mut self, addr: WindowAddr) -> Vec<Action> {
        self.active_window = Some(addr);
        let previous = self.active_layout;
        let layer_focused = !self.focused_layers.is_empty();
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

        if layer_focused {
            log::debug!(
                "layout switch deferred: address={addr} target_layout={target} layer still focused"
            );
            return Vec::new();
        }

        self.switch_actions(target)
    }

    fn apply_layer_event(&mut self, namespace: &str, opening: bool) -> Vec<Action> {
        let mut live_layers = self.focused_layers.clone();
        if opening {
            live_layers.push(namespace.to_string());
        } else if let Some(position) = live_layers.iter().rposition(|known| known == namespace) {
            live_layers.remove(position);
        }
        self.sync_layers(&live_layers)
    }

    pub fn sync_layers(&mut self, live_layers: &[String]) -> Vec<Action> {
        let previous = self.focused_layers.clone();
        let layer_target = self.reconcile_layers(live_layers, None);
        if self.focused_layers == previous {
            return Vec::new();
        }

        let target = layer_target.or_else(|| self.active_window_layout());
        let Some(target) = target else {
            log::debug!("layer snapshot changed with nothing to restore");
            return Vec::new();
        };
        log::debug!("layer snapshot reconciled: target_layout={target}");
        self.switch_actions(target)
    }

    fn layer_default(&self, namespace: &str) -> Option<LayoutIndex> {
        if let Some(layout) = self.layer_defaults.get(namespace) {
            return Some(*layout);
        }
        self.layer_patterns
            .iter()
            .find(|(pattern, _)| pattern.is_match(namespace))
            .map(|(_, layout)| *layout)
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
        } else {
            self.seed_keyboard(keyboard);
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
        if let Some(layout) = self.defaults_by_class.get(class_name) {
            return *layout;
        }
        self.default_patterns
            .iter()
            .find(|(pattern, _)| pattern.is_match(class_name))
            .map(|(_, layout)| *layout)
            .unwrap_or(0)
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

    fn reconcile_layers(
        &mut self,
        live_layers: &[String],
        adopt_unknown_top: Option<LayoutIndex>,
    ) -> Option<LayoutIndex> {
        self.reconcile_focused_layers(live_layers);
        let focused = self.focused_layers.clone();
        let top = focused.last()?.clone();
        let top_was_known = self.layers.contains_key(&top);

        for namespace in focused {
            if !self.layers.contains_key(&namespace) {
                let default = self
                    .layer_default(&namespace)
                    .expect("focused layers were filtered through layer_default");
                self.layers.insert(namespace, default);
            }
        }

        if !top_was_known {
            if let Some(layout) = adopt_unknown_top {
                self.layers.insert(top.clone(), layout);
            }
        }
        self.layers.get(&top).copied()
    }

    fn reconcile_focused_layers(&mut self, live_layers: &[String]) {
        let tracked: Vec<String> = live_layers
            .iter()
            .filter(|namespace| self.layer_default(namespace).is_some())
            .cloned()
            .collect();
        let mut remaining = HashMap::<String, usize>::new();
        for namespace in &tracked {
            *remaining.entry(namespace.clone()).or_default() += 1;
        }

        let mut reconciled = Vec::with_capacity(tracked.len());
        for namespace in &self.focused_layers {
            let Some(count) = remaining.get_mut(namespace) else {
                continue;
            };
            if *count > 0 {
                reconciled.push(namespace.clone());
                *count -= 1;
            }
        }
        for namespace in tracked {
            let count = remaining
                .get_mut(&namespace)
                .expect("tracked namespace has a count");
            if *count > 0 {
                reconciled.push(namespace);
                *count -= 1;
            }
        }
        self.focused_layers = reconciled;
    }
}

// Every configured key is also usable as an anchored regex, matched only
// when no exact entry fits. Sorted by pattern text so overlapping patterns
// resolve the same way on every run. Keys that fail to compile (stray
// parens etc.) still work as exact matches, so this never rejects a config.
fn compile_patterns(
    defaults: &HashMap<String, LayoutIndex>,
    section: &str,
) -> Vec<(Regex, LayoutIndex)> {
    let mut entries: Vec<(&String, LayoutIndex)> = defaults
        .iter()
        .map(|(key, layout)| (key, *layout))
        .collect();
    entries.sort();
    entries
        .into_iter()
        .filter_map(|(key, layout)| match Regex::new(&format!("^(?:{key})$")) {
            Ok(pattern) => Some((pattern, layout)),
            Err(err) => {
                log::debug!("{section} key {key:?} is not a valid regex ({err}); exact match only");
                None
            }
        })
        .collect()
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
            layer_layouts: HashMap::new(),
        }
    }

    fn layer_config(layers: &[(&str, LayoutIndex)]) -> Config {
        Config {
            layer_layouts: layers
                .iter()
                .map(|(namespace, layout)| ((*namespace).to_string(), *layout))
                .collect(),
            ..config(&["kbd"], &[])
        }
    }

    fn switch(layout: LayoutIndex, previous: LayoutIndex) -> Vec<Action> {
        vec![Action::SwitchLayout {
            keyboards: vec!["kbd".to_string()],
            layout,
            previous,
        }]
    }

    fn layer_snapshot(namespaces: &[&str]) -> Vec<String> {
        namespaces
            .iter()
            .map(|namespace| (*namespace).to_string())
            .collect()
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
    fn new_window_matches_class_default_as_regex() {
        let mut state = RuntimeState::new(config(&["kbd"], &[("chrome-.*whatsapp.*", 1)]), 0);
        state.handle_event(Event::ActiveWindow {
            class_name: "chrome-web.whatsapp.com__-Default",
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
    fn exact_class_default_wins_over_pattern() {
        let mut state = RuntimeState::new(config(&["kbd"], &[("fire.*", 1), ("firefox", 2)]), 0);
        state.handle_event(Event::ActiveWindow {
            class_name: "firefox",
        });
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });

        assert_eq!(state.active_window_layout(), Some(2));
    }

    #[test]
    fn pattern_matches_whole_class_only() {
        let mut state = RuntimeState::new(config(&["kbd"], &[("firefox", 1)]), 0);
        state.handle_event(Event::ActiveWindow {
            class_name: "firefox-nightly",
        });
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });

        assert_eq!(state.active_window_layout(), Some(0));
    }

    #[test]
    fn invalid_regex_key_still_matches_exactly() {
        let mut state = RuntimeState::new(config(&["kbd"], &[("app(broken", 1)]), 0);
        state.handle_event(Event::ActiveWindow {
            class_name: "app(broken",
        });
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });

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
    fn untracked_layer_is_ignored() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 0)]), 1);
        assert!(!state.should_reconcile_layer_event("waybar"));
        assert!(state.sync_layers(&layer_snapshot(&["waybar"])).is_empty());
        assert!(state.sync_layers(&[]).is_empty());
        assert_eq!(state.active_layout(), 1);
    }

    #[test]
    fn tracked_layer_applies_default_then_restores_window() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 0)]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);

        assert_eq!(state.sync_layers(&layer_snapshot(&["rofi"])), switch(0, 1));
        assert_eq!(state.sync_layers(&[]), switch(1, 0));
    }

    #[test]
    fn layer_remembers_its_own_layout_across_opens() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 0)]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.sync_layers(&layer_snapshot(&["rofi"]));

        state.record_keyboard_layout("kbd", 1);
        assert_eq!(state.active_window_layout(), Some(0));

        assert_eq!(state.sync_layers(&[]), switch(0, 1));
        assert_eq!(state.sync_layers(&layer_snapshot(&["rofi"])), switch(1, 0));
    }

    #[test]
    fn layer_namespace_matches_as_regex() {
        let mut state = RuntimeState::new(layer_config(&[("rofi.*", 1)]), 0);
        assert_eq!(
            state.sync_layers(&layer_snapshot(&["rofi-launcher"])),
            switch(1, 0)
        );
    }

    #[test]
    fn window_focus_under_a_layer_does_not_switch_until_it_closes() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 0)]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);
        state.sync_layers(&layer_snapshot(&["rofi"]));

        // Hyprland re-announces the focused window around a layer grab.
        assert!(state
            .handle_event(Event::ActiveWindowV2 {
                addr: WindowAddr(1)
            })
            .is_empty());
        assert_eq!(state.active_layout(), 0);

        assert_eq!(state.sync_layers(&[]), switch(1, 0));
    }

    #[test]
    fn stacked_layers_unwind_in_order() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 0), ("menu", 2)]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);

        assert_eq!(state.sync_layers(&layer_snapshot(&["rofi"])), switch(0, 1));
        assert_eq!(
            state.sync_layers(&layer_snapshot(&["rofi", "menu"])),
            switch(2, 0)
        );
        assert_eq!(state.sync_layers(&layer_snapshot(&["rofi"])), switch(0, 2));
        assert_eq!(state.sync_layers(&[]), switch(1, 0));
    }

    #[test]
    fn same_namespace_layers_unwind_one_surface_at_a_time() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 0)]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);

        assert_eq!(state.sync_layers(&layer_snapshot(&["rofi"])), switch(0, 1));
        assert!(state
            .sync_layers(&layer_snapshot(&["rofi", "rofi"]))
            .is_empty());

        // One rofi surface remains mapped, so the window must not be restored.
        assert!(state.sync_layers(&layer_snapshot(&["rofi"])).is_empty());
        assert_eq!(state.active_layout(), 0);

        assert_eq!(state.sync_layers(&[]), switch(1, 0));
    }

    #[test]
    fn raw_layer_events_match_snapshot_transitions() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 0)]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);

        assert_eq!(
            state.handle_event(Event::OpenLayer { namespace: "rofi" }),
            switch(0, 1)
        );
        assert!(state.should_reconcile_layer_event("waybar"));
        assert_eq!(
            state.handle_event(Event::CloseLayer { namespace: "rofi" }),
            switch(1, 0)
        );
    }

    #[test]
    fn seeded_keyboard_switches_before_first_layout_event() {
        let mut cfg = layer_config(&[("rofi", 1)]);
        cfg.keyboards.include.clear();
        let mut state = RuntimeState::new(cfg, 0);
        state.seed_keyboard("kbd");

        assert_eq!(state.sync_layers(&layer_snapshot(&["rofi"])), switch(1, 0));
    }

    #[test]
    fn unmatched_close_layer_is_ignored() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 1)]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });

        assert!(state.sync_layers(&[]).is_empty());
        assert_eq!(state.active_layout(), 0);
    }

    #[test]
    fn resync_drops_stranded_layer_focus() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 1)]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.sync_layers(&layer_snapshot(&["rofi"]));

        state.resync(
            Some((WindowAddr(1), "firefox".to_string())),
            1,
            &[WindowAddr(1)],
            &[],
        );

        // Focus is on the window again, so layout changes land on the window.
        state.record_keyboard_layout("kbd", 1);
        assert_eq!(state.active_window_layout(), Some(1));
    }

    #[test]
    fn resync_keeps_layer_focus_while_it_remains_mapped() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 0)]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);
        state.sync_layers(&layer_snapshot(&["rofi"]));

        assert!(state
            .resync(
                Some((WindowAddr(1), "firefox".to_string())),
                0,
                &[WindowAddr(1)],
                &["rofi".to_string()],
            )
            .is_empty());

        state.record_keyboard_layout("kbd", 2);
        assert_eq!(state.active_window_layout(), Some(1));
        assert_eq!(state.sync_layers(&[]), switch(1, 2));
    }

    #[test]
    fn resync_adopts_startup_layer_without_polluting_window() {
        let mut state = RuntimeState::new(layer_config(&[("rofi", 1)]), 0);

        assert!(state
            .resync(
                Some((WindowAddr(1), "codium".to_string())),
                0,
                &[WindowAddr(1)],
                &["rofi".to_string()],
            )
            .is_empty());
        assert_eq!(state.active_window_layout(), Some(0));
        // A buffered open event after connect must not double-count the layer.
        assert!(state.sync_layers(&layer_snapshot(&["rofi"])).is_empty());

        state.record_keyboard_layout("kbd", 1);
        assert_eq!(state.active_window_layout(), Some(0));
        assert_eq!(state.sync_layers(&[]), switch(0, 1));
    }

    #[test]
    fn resync_prunes_closed_windows() {
        let mut state = RuntimeState::new(config(&["kbd"], &[]), 0);
        state.handle_event(Event::ActiveWindowV2 {
            addr: WindowAddr(1),
        });
        state.record_keyboard_layout("kbd", 1);

        assert!(state.resync(None, 0, &[], &[]).is_empty());

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
                &[WindowAddr(1)],
                &[]
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
                &[WindowAddr(7)],
                &[]
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
            &[],
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
