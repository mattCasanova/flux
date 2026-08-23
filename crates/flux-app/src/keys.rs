//! Keybinding config (F15) — `[keys]` in flux.toml.
//!
//! Actions are named (`new_tab`, `find`, `undo`, …); each maps to one
//! chord string like `"cmd+shift+z"` or `"pageup"`. Modifier names:
//! `cmd` / `super`, `ctrl`, `alt` / `opt`, `shift`. Keys: a single
//! character, or `up` / `down` / `left` / `right` / `enter` / `tab` /
//! `escape` / `backspace` / `delete` / `home` / `end` / `pageup` /
//! `pagedown` / `space` / `f1`..`f12`. Bindings the config leaves out
//! keep their defaults; `""` unbinds an action; unknown action names or
//! unparsable chords are logged and ignored (never a hard failure).
//!
//! The keymap only covers Flux-level shortcuts. Editing keys inside the
//! input bar (arrows, Home/End, Backspace, Enter, Tab-for-completion)
//! and everything forwarded to the PTY are not configurable here.

use std::collections::HashMap;

use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Everything a chord can trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    /// Jump to tab N (1-based). Not configurable per tab; `tab_1`…
    /// `tab_9` in config rebind the chord for each N.
    Tab(u8),
    Find,
    Copy,
    CopyBlockOutput,
    Paste,
    Undo,
    Redo,
    ScrollLineUp,
    ScrollLineDown,
    ScrollPageUp,
    ScrollPageDown,
    PrevBlock,
    NextBlock,
    SplitRight,
    SplitDown,
    ClosePane,
    FocusPaneLeft,
    FocusPaneRight,
    FocusPaneUp,
    FocusPaneDown,
}

impl Action {
    /// Runs before any popup gets the key: tab management and find
    /// must work while the search bar or vim owns the keyboard.
    pub fn is_global(self) -> bool {
        matches!(
            self,
            Action::NewTab
                | Action::CloseTab
                | Action::NextTab
                | Action::PrevTab
                | Action::Tab(_)
                | Action::Find
                | Action::SplitRight
                | Action::SplitDown
                | Action::ClosePane
                | Action::FocusPaneLeft
                | Action::FocusPaneRight
                | Action::FocusPaneUp
                | Action::FocusPaneDown
        )
    }

    /// May fire while the PTY owns the keyboard (alt screen / running
    /// command). Scrolling, undo and block hops are editor-side and
    /// would steal keys vim wants (PageUp!), so they only fire at the
    /// prompt; clipboard, find and tabs work everywhere.
    pub fn allowed_in_raw(self) -> bool {
        self.is_global() || matches!(self, Action::Copy | Action::CopyBlockOutput | Action::Paste)
    }

    fn name(self) -> String {
        match self {
            Action::NewTab => "new_tab".into(),
            Action::CloseTab => "close_tab".into(),
            Action::NextTab => "next_tab".into(),
            Action::PrevTab => "prev_tab".into(),
            Action::Tab(n) => format!("tab_{n}"),
            Action::Find => "find".into(),
            Action::Copy => "copy".into(),
            Action::CopyBlockOutput => "copy_block_output".into(),
            Action::Paste => "paste".into(),
            Action::Undo => "undo".into(),
            Action::Redo => "redo".into(),
            Action::ScrollLineUp => "scroll_line_up".into(),
            Action::ScrollLineDown => "scroll_line_down".into(),
            Action::ScrollPageUp => "scroll_page_up".into(),
            Action::ScrollPageDown => "scroll_page_down".into(),
            Action::PrevBlock => "prev_block".into(),
            Action::NextBlock => "next_block".into(),
            Action::SplitRight => "split_right".into(),
            Action::SplitDown => "split_down".into(),
            Action::ClosePane => "close_pane".into(),
            Action::FocusPaneLeft => "focus_pane_left".into(),
            Action::FocusPaneRight => "focus_pane_right".into(),
            Action::FocusPaneUp => "focus_pane_up".into(),
            Action::FocusPaneDown => "focus_pane_down".into(),
        }
    }

    fn from_name(name: &str) -> Option<Action> {
        Some(match name {
            "new_tab" => Action::NewTab,
            "close_tab" => Action::CloseTab,
            "next_tab" => Action::NextTab,
            "prev_tab" => Action::PrevTab,
            "find" => Action::Find,
            "copy" => Action::Copy,
            "copy_block_output" => Action::CopyBlockOutput,
            "paste" => Action::Paste,
            "undo" => Action::Undo,
            "redo" => Action::Redo,
            "scroll_line_up" => Action::ScrollLineUp,
            "scroll_line_down" => Action::ScrollLineDown,
            "scroll_page_up" => Action::ScrollPageUp,
            "scroll_page_down" => Action::ScrollPageDown,
            "prev_block" => Action::PrevBlock,
            "next_block" => Action::NextBlock,
            "split_right" => Action::SplitRight,
            "split_down" => Action::SplitDown,
            "close_pane" => Action::ClosePane,
            "focus_pane_left" => Action::FocusPaneLeft,
            "focus_pane_right" => Action::FocusPaneRight,
            "focus_pane_up" => Action::FocusPaneUp,
            "focus_pane_down" => Action::FocusPaneDown,
            _ => {
                let n: u8 = name.strip_prefix("tab_")?.parse().ok()?;
                if (1..=9).contains(&n) {
                    Action::Tab(n)
                } else {
                    return None;
                }
            }
        })
    }

    #[cfg(test)]
    fn all() -> Vec<Action> {
        let mut all = vec![
            Action::NewTab,
            Action::CloseTab,
            Action::NextTab,
            Action::PrevTab,
            Action::Find,
            Action::Copy,
            Action::CopyBlockOutput,
            Action::Paste,
            Action::Undo,
            Action::Redo,
            Action::ScrollLineUp,
            Action::ScrollLineDown,
            Action::ScrollPageUp,
            Action::ScrollPageDown,
            Action::PrevBlock,
            Action::NextBlock,
            Action::SplitRight,
            Action::SplitDown,
            Action::ClosePane,
            Action::FocusPaneLeft,
            Action::FocusPaneRight,
            Action::FocusPaneUp,
            Action::FocusPaneDown,
        ];
        all.extend((1..=9).map(Action::Tab));
        all
    }
}

/// The key half of a chord.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeySpec {
    /// A character key, lowercased.
    Char(char),
    Named(NamedKey),
}

/// A modifier set + key. Modifiers must match exactly (Cmd+Shift+Z is
/// not Cmd+Z), so a binding never fires on a superset chord.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chord {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: KeySpec,
}

impl Chord {
    /// Parse `"cmd+shift+z"`. Order-insensitive; case-insensitive.
    pub fn parse(text: &str) -> Result<Chord, String> {
        let mut chord = Chord {
            cmd: false,
            ctrl: false,
            alt: false,
            shift: false,
            key: KeySpec::Char(' '),
        };
        let mut key: Option<KeySpec> = None;
        // A literal '+' key is written as the last token: "cmd++".
        let parts: Vec<&str> = if let Some(prefix) = text.strip_suffix("++") {
            let mut p: Vec<&str> = prefix.split('+').collect();
            p.push("+");
            p
        } else {
            text.split('+').collect()
        };
        for raw in parts {
            let part = raw.trim().to_ascii_lowercase();
            match part.as_str() {
                "cmd" | "super" | "command" | "meta" => chord.cmd = true,
                "ctrl" | "control" => chord.ctrl = true,
                "alt" | "opt" | "option" => chord.alt = true,
                "shift" => chord.shift = true,
                "" => return Err(format!("empty token in chord {text:?}")),
                _ => {
                    if key.is_some() {
                        return Err(format!("chord {text:?} has two keys"));
                    }
                    key = Some(
                        parse_key(&part)
                            .ok_or_else(|| format!("unknown key {part:?} in {text:?}"))?,
                    );
                }
            }
        }
        chord.key = key.ok_or_else(|| format!("chord {text:?} has no key"))?;
        Ok(chord)
    }

    /// The chord a key event represents, if it is bindable at all.
    pub fn from_event(key: &Key, mods: ModifiersState) -> Option<Chord> {
        let key = match key {
            Key::Character(s) => {
                let mut chars = s.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                KeySpec::Char(c.to_ascii_lowercase())
            }
            Key::Named(NamedKey::Space) => KeySpec::Char(' '),
            Key::Named(named) => KeySpec::Named(*named),
            _ => return None,
        };
        Some(Chord {
            cmd: mods.super_key(),
            ctrl: mods.control_key(),
            alt: mods.alt_key(),
            shift: mods.shift_key(),
            key,
        })
    }
}

fn parse_key(part: &str) -> Option<KeySpec> {
    let mut chars = part.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(KeySpec::Char(c));
    }
    let named = match part {
        "up" => NamedKey::ArrowUp,
        "down" => NamedKey::ArrowDown,
        "left" => NamedKey::ArrowLeft,
        "right" => NamedKey::ArrowRight,
        "enter" | "return" => NamedKey::Enter,
        "tab" => NamedKey::Tab,
        "escape" | "esc" => NamedKey::Escape,
        "backspace" => NamedKey::Backspace,
        "delete" | "del" => NamedKey::Delete,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" => NamedKey::PageUp,
        "pagedown" => NamedKey::PageDown,
        "space" => return Some(KeySpec::Char(' ')),
        "f1" => NamedKey::F1,
        "f2" => NamedKey::F2,
        "f3" => NamedKey::F3,
        "f4" => NamedKey::F4,
        "f5" => NamedKey::F5,
        "f6" => NamedKey::F6,
        "f7" => NamedKey::F7,
        "f8" => NamedKey::F8,
        "f9" => NamedKey::F9,
        "f10" => NamedKey::F10,
        "f11" => NamedKey::F11,
        "f12" => NamedKey::F12,
        _ => return None,
    };
    Some(KeySpec::Named(named))
}

/// Chord → action lookup, built from defaults + `[keys]` overrides.
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: HashMap<Chord, Action>,
}

impl Keymap {
    /// Platform defaults: Cmd-based on macOS, Ctrl(+Shift) elsewhere.
    pub fn defaults() -> Self {
        let mac = cfg!(target_os = "macos");
        let primary = if mac { "cmd" } else { "ctrl+shift" };
        let edit = if mac { "cmd" } else { "ctrl" };
        let mut map = Keymap {
            bindings: HashMap::new(),
        };
        let mut bind = |action: Action, chord: String| {
            let chord = Chord::parse(&chord).expect("default chord parses");
            map.bindings.insert(chord, action);
        };
        bind(Action::NewTab, format!("{primary}+t"));
        bind(Action::CloseTab, format!("{primary}+w"));
        bind(Action::NextTab, format!("{primary}+]"));
        bind(Action::PrevTab, format!("{primary}+["));
        for n in 1..=9u8 {
            bind(Action::Tab(n), format!("{primary}+{n}"));
        }
        bind(Action::Find, format!("{primary}+f"));
        bind(Action::Copy, format!("{primary}+c"));
        bind(Action::CopyBlockOutput, format!("{edit}+shift+c"));
        bind(Action::Paste, format!("{primary}+v"));
        bind(Action::Undo, format!("{edit}+z"));
        bind(Action::Redo, format!("{edit}+shift+z"));
        bind(Action::ScrollLineUp, "alt+up".into());
        bind(Action::ScrollLineDown, "alt+down".into());
        bind(Action::ScrollPageUp, "pageup".into());
        bind(Action::ScrollPageDown, "pagedown".into());
        bind(Action::PrevBlock, format!("{edit}+up"));
        bind(Action::NextBlock, format!("{edit}+down"));
        bind(Action::SplitRight, format!("{primary}+d"));
        bind(Action::SplitDown, format!("{primary}+shift+d"));
        bind(Action::ClosePane, format!("{primary}+shift+w"));
        bind(Action::FocusPaneLeft, format!("{primary}+alt+left"));
        bind(Action::FocusPaneRight, format!("{primary}+alt+right"));
        bind(Action::FocusPaneUp, format!("{primary}+alt+up"));
        bind(Action::FocusPaneDown, format!("{primary}+alt+down"));
        map
    }

    /// Apply `[keys]` overrides. Each entry rebinds one action; an
    /// empty string unbinds it. Problems are logged, never fatal.
    pub fn with_overrides(mut self, overrides: &HashMap<String, String>) -> Self {
        for (name, chord_text) in overrides {
            let Some(action) = Action::from_name(name) else {
                log::warn!("[keys] unknown action {name:?}; ignoring");
                continue;
            };
            let unbind = chord_text.trim().is_empty();
            let parsed = if unbind {
                None
            } else {
                match Chord::parse(chord_text) {
                    Ok(chord) => Some(chord),
                    Err(e) => {
                        log::warn!("[keys] {name}: {e}; keeping default");
                        continue;
                    }
                }
            };
            // Only now drop the action's current binding(s).
            self.bindings.retain(|_, a| *a != action);
            if let Some(chord) = parsed
                && let Some(prev) = self.bindings.insert(chord, action)
            {
                log::warn!(
                    "[keys] {name} = {chord_text:?} takes over the chord from {}",
                    prev.name()
                );
            }
        }
        self
    }

    pub fn action_for(&self, key: &Key, mods: ModifiersState) -> Option<Action> {
        let chord = Chord::from_event(key, mods)?;
        self.bindings.get(&chord).copied()
    }

    /// Chord text for an action, for help/UI. None if unbound.
    #[allow(dead_code)] // help overlay / docs generation later
    pub fn chord_of(&self, action: Action) -> Option<String> {
        self.bindings
            .iter()
            .find(|(_, a)| **a == action)
            .map(|(c, _)| chord_to_string(c))
    }

    #[cfg(test)]
    fn all_actions_bound(&self) -> bool {
        Action::all().iter().all(|a| self.chord_of(*a).is_some())
    }
}

#[allow(dead_code)]
fn chord_to_string(chord: &Chord) -> String {
    let mut parts: Vec<String> = Vec::new();
    if chord.cmd {
        parts.push("cmd".into());
    }
    if chord.ctrl {
        parts.push("ctrl".into());
    }
    if chord.alt {
        parts.push("alt".into());
    }
    if chord.shift {
        parts.push("shift".into());
    }
    parts.push(match &chord.key {
        KeySpec::Char(' ') => "space".into(),
        KeySpec::Char(c) => c.to_string(),
        KeySpec::Named(n) => format!("{n:?}").to_ascii_lowercase(),
    });
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(cmd: bool, ctrl: bool, alt: bool, shift: bool) -> ModifiersState {
        let mut m = ModifiersState::empty();
        if cmd {
            m |= ModifiersState::SUPER;
        }
        if ctrl {
            m |= ModifiersState::CONTROL;
        }
        if alt {
            m |= ModifiersState::ALT;
        }
        if shift {
            m |= ModifiersState::SHIFT;
        }
        m
    }

    #[test]
    fn parses_chords_in_any_order_and_case() {
        let a = Chord::parse("Cmd+Shift+Z").unwrap();
        let b = Chord::parse("shift+cmd+z").unwrap();
        assert_eq!(a, b);
        assert!(a.cmd && a.shift && !a.ctrl);
        assert_eq!(a.key, KeySpec::Char('z'));
        assert_eq!(
            Chord::parse("pageup").unwrap().key,
            KeySpec::Named(NamedKey::PageUp)
        );
        assert_eq!(Chord::parse("cmd++").unwrap().key, KeySpec::Char('+'));
        assert!(Chord::parse("cmd+").is_err());
        assert!(Chord::parse("cmd+bogus").is_err());
        assert!(Chord::parse("cmd+a+b").is_err());
    }

    #[test]
    fn defaults_bind_every_action_and_resolve_events() {
        let map = Keymap::defaults();
        assert!(map.all_actions_bound());
        let primary = mods(
            cfg!(target_os = "macos"),
            !cfg!(target_os = "macos"),
            false,
            !cfg!(target_os = "macos"),
        );
        assert_eq!(
            map.action_for(&Key::Character("t".into()), primary),
            Some(Action::NewTab)
        );
        // Shift+Cmd+T is NOT new_tab — modifiers match exactly.
        let mut plus_shift = primary;
        plus_shift |= ModifiersState::SHIFT;
        if cfg!(target_os = "macos") {
            assert_eq!(
                map.action_for(&Key::Character("t".into()), plus_shift),
                None
            );
        }
        assert_eq!(
            map.action_for(&Key::Named(NamedKey::PageUp), ModifiersState::empty()),
            Some(Action::ScrollPageUp)
        );
    }

    #[test]
    fn overrides_rebind_unbind_and_ignore_garbage() {
        let mut o = HashMap::new();
        o.insert("find".to_string(), "ctrl+alt+f".to_string());
        o.insert("scroll_page_up".to_string(), "".to_string());
        o.insert("not_an_action".to_string(), "cmd+q".to_string());
        o.insert("undo".to_string(), "cmd+".to_string()); // unparsable
        let map = Keymap::defaults().with_overrides(&o);
        assert_eq!(
            map.action_for(&Key::Character("f".into()), mods(false, true, true, false)),
            Some(Action::Find)
        );
        assert_eq!(map.chord_of(Action::ScrollPageUp), None, "unbound");
        assert!(
            map.chord_of(Action::Undo).is_some(),
            "bad chord keeps default"
        );
        // The old find chord is gone.
        let primary = mods(
            cfg!(target_os = "macos"),
            !cfg!(target_os = "macos"),
            false,
            !cfg!(target_os = "macos"),
        );
        assert_eq!(map.action_for(&Key::Character("f".into()), primary), None);
    }
}
