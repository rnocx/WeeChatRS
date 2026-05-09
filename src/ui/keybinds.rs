use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AppKey {
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    A, B, C, D, E, F, G, H, I, J, K, L, M,
    N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9,
    Space, Enter, Tab, Escape,
    PageUp, PageDown, Home, End,
    Insert, Delete, Backspace,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
}

impl AppKey {
    pub fn to_egui(self) -> egui::Key {
        match self {
            Self::ArrowUp    => egui::Key::ArrowUp,
            Self::ArrowDown  => egui::Key::ArrowDown,
            Self::ArrowLeft  => egui::Key::ArrowLeft,
            Self::ArrowRight => egui::Key::ArrowRight,
            Self::A => egui::Key::A, Self::B => egui::Key::B,
            Self::C => egui::Key::C, Self::D => egui::Key::D,
            Self::E => egui::Key::E, Self::F => egui::Key::F,
            Self::G => egui::Key::G, Self::H => egui::Key::H,
            Self::I => egui::Key::I, Self::J => egui::Key::J,
            Self::K => egui::Key::K, Self::L => egui::Key::L,
            Self::M => egui::Key::M, Self::N => egui::Key::N,
            Self::O => egui::Key::O, Self::P => egui::Key::P,
            Self::Q => egui::Key::Q, Self::R => egui::Key::R,
            Self::S => egui::Key::S, Self::T => egui::Key::T,
            Self::U => egui::Key::U, Self::V => egui::Key::V,
            Self::W => egui::Key::W, Self::X => egui::Key::X,
            Self::Y => egui::Key::Y, Self::Z => egui::Key::Z,
            Self::Num0 => egui::Key::Num0, Self::Num1 => egui::Key::Num1,
            Self::Num2 => egui::Key::Num2, Self::Num3 => egui::Key::Num3,
            Self::Num4 => egui::Key::Num4, Self::Num5 => egui::Key::Num5,
            Self::Num6 => egui::Key::Num6, Self::Num7 => egui::Key::Num7,
            Self::Num8 => egui::Key::Num8, Self::Num9 => egui::Key::Num9,
            Self::Space     => egui::Key::Space,
            Self::Enter     => egui::Key::Enter,
            Self::Tab       => egui::Key::Tab,
            Self::Escape    => egui::Key::Escape,
            Self::PageUp    => egui::Key::PageUp,
            Self::PageDown  => egui::Key::PageDown,
            Self::Home      => egui::Key::Home,
            Self::End       => egui::Key::End,
            Self::Insert    => egui::Key::Insert,
            Self::Delete    => egui::Key::Delete,
            Self::Backspace => egui::Key::Backspace,
            Self::F1  => egui::Key::F1,  Self::F2  => egui::Key::F2,
            Self::F3  => egui::Key::F3,  Self::F4  => egui::Key::F4,
            Self::F5  => egui::Key::F5,  Self::F6  => egui::Key::F6,
            Self::F7  => egui::Key::F7,  Self::F8  => egui::Key::F8,
            Self::F9  => egui::Key::F9,  Self::F10 => egui::Key::F10,
            Self::F11 => egui::Key::F11, Self::F12 => egui::Key::F12,
        }
    }

    pub fn from_egui(key: egui::Key) -> Option<Self> {
        match key {
            egui::Key::ArrowUp    => Some(Self::ArrowUp),
            egui::Key::ArrowDown  => Some(Self::ArrowDown),
            egui::Key::ArrowLeft  => Some(Self::ArrowLeft),
            egui::Key::ArrowRight => Some(Self::ArrowRight),
            egui::Key::A => Some(Self::A), egui::Key::B => Some(Self::B),
            egui::Key::C => Some(Self::C), egui::Key::D => Some(Self::D),
            egui::Key::E => Some(Self::E), egui::Key::F => Some(Self::F),
            egui::Key::G => Some(Self::G), egui::Key::H => Some(Self::H),
            egui::Key::I => Some(Self::I), egui::Key::J => Some(Self::J),
            egui::Key::K => Some(Self::K), egui::Key::L => Some(Self::L),
            egui::Key::M => Some(Self::M), egui::Key::N => Some(Self::N),
            egui::Key::O => Some(Self::O), egui::Key::P => Some(Self::P),
            egui::Key::Q => Some(Self::Q), egui::Key::R => Some(Self::R),
            egui::Key::S => Some(Self::S), egui::Key::T => Some(Self::T),
            egui::Key::U => Some(Self::U), egui::Key::V => Some(Self::V),
            egui::Key::W => Some(Self::W), egui::Key::X => Some(Self::X),
            egui::Key::Y => Some(Self::Y), egui::Key::Z => Some(Self::Z),
            egui::Key::Num0 => Some(Self::Num0), egui::Key::Num1 => Some(Self::Num1),
            egui::Key::Num2 => Some(Self::Num2), egui::Key::Num3 => Some(Self::Num3),
            egui::Key::Num4 => Some(Self::Num4), egui::Key::Num5 => Some(Self::Num5),
            egui::Key::Num6 => Some(Self::Num6), egui::Key::Num7 => Some(Self::Num7),
            egui::Key::Num8 => Some(Self::Num8), egui::Key::Num9 => Some(Self::Num9),
            egui::Key::Space     => Some(Self::Space),
            egui::Key::Enter     => Some(Self::Enter),
            egui::Key::Tab       => Some(Self::Tab),
            egui::Key::Escape    => Some(Self::Escape),
            egui::Key::PageUp    => Some(Self::PageUp),
            egui::Key::PageDown  => Some(Self::PageDown),
            egui::Key::Home      => Some(Self::Home),
            egui::Key::End       => Some(Self::End),
            egui::Key::Insert    => Some(Self::Insert),
            egui::Key::Delete    => Some(Self::Delete),
            egui::Key::Backspace => Some(Self::Backspace),
            egui::Key::F1  => Some(Self::F1),  egui::Key::F2  => Some(Self::F2),
            egui::Key::F3  => Some(Self::F3),  egui::Key::F4  => Some(Self::F4),
            egui::Key::F5  => Some(Self::F5),  egui::Key::F6  => Some(Self::F6),
            egui::Key::F7  => Some(Self::F7),  egui::Key::F8  => Some(Self::F8),
            egui::Key::F9  => Some(Self::F9),  egui::Key::F10 => Some(Self::F10),
            egui::Key::F11 => Some(Self::F11), egui::Key::F12 => Some(Self::F12),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ArrowUp    => "↑",
            Self::ArrowDown  => "↓",
            Self::ArrowLeft  => "←",
            Self::ArrowRight => "→",
            Self::A => "A", Self::B => "B", Self::C => "C", Self::D => "D",
            Self::E => "E", Self::F => "F", Self::G => "G", Self::H => "H",
            Self::I => "I", Self::J => "J", Self::K => "K", Self::L => "L",
            Self::M => "M", Self::N => "N", Self::O => "O", Self::P => "P",
            Self::Q => "Q", Self::R => "R", Self::S => "S", Self::T => "T",
            Self::U => "U", Self::V => "V", Self::W => "W", Self::X => "X",
            Self::Y => "Y", Self::Z => "Z",
            Self::Num0 => "0", Self::Num1 => "1", Self::Num2 => "2",
            Self::Num3 => "3", Self::Num4 => "4", Self::Num5 => "5",
            Self::Num6 => "6", Self::Num7 => "7", Self::Num8 => "8",
            Self::Num9 => "9",
            Self::Space     => "Space",
            Self::Enter     => "Enter",
            Self::Tab       => "Tab",
            Self::Escape    => "Esc",
            Self::PageUp    => "PgUp",
            Self::PageDown  => "PgDn",
            Self::Home      => "Home",
            Self::End       => "End",
            Self::Insert    => "Ins",
            Self::Delete    => "Del",
            Self::Backspace => "Bksp",
            Self::F1  => "F1",  Self::F2  => "F2",  Self::F3  => "F3",
            Self::F4  => "F4",  Self::F5  => "F5",  Self::F6  => "F6",
            Self::F7  => "F7",  Self::F8  => "F8",  Self::F9  => "F9",
            Self::F10 => "F10", Self::F11 => "F11", Self::F12 => "F12",
        }
    }
}

/// Platform-aware display of modifiers.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct SerdeModifiers {
    /// Ctrl on all platforms
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Maps to Cmd (⌘) on macOS, Ctrl on Windows/Linux via egui's command abstraction
    pub command: bool,
}

impl SerdeModifiers {
    pub fn to_egui(self) -> egui::Modifiers {
        // On Windows/Linux, egui reports ctrl=true whenever Ctrl is pressed (same key as
        // "command"). We must set ctrl=true here so consume_key's exact-match works.
        let ctrl = self.ctrl || (self.command && !cfg!(target_os = "macos"));
        egui::Modifiers {
            alt: self.alt,
            ctrl,
            shift: self.shift,
            mac_cmd: false,
            command: self.command,
        }
    }

    pub fn from_egui(m: egui::Modifiers) -> Self {
        let command = m.command || m.mac_cmd;
        Self {
            alt: m.alt,
            // On Windows/Linux, Ctrl sets both ctrl and command. Normalise: if command
            // already covers the Ctrl intent, don't store ctrl separately.
            ctrl: m.ctrl && !command,
            shift: m.shift,
            command,
        }
    }

    pub fn is_empty(self) -> bool {
        !self.ctrl && !self.alt && !self.shift && !self.command
    }

    pub fn display(self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.command {
            #[cfg(target_os = "macos")]
            parts.push("⌘");
            #[cfg(not(target_os = "macos"))]
            parts.push("Ctrl");
        }
        if self.ctrl && !self.command {
            parts.push("Ctrl");
        }
        if self.alt {
            #[cfg(target_os = "macos")]
            parts.push("⌥");
            #[cfg(not(target_os = "macos"))]
            parts.push("Alt");
        }
        if self.shift {
            #[cfg(target_os = "macos")]
            parts.push("⇧");
            #[cfg(not(target_os = "macos"))]
            parts.push("Shift");
        }
        parts.join("+")
    }
}

/// Actions that can be bound to keyboard shortcuts.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeybindAction {
    CycleBufferUp,
    CycleBufferDown,
    ToggleSearch,
    ToggleBufferList,
    ToggleNicklist,
    ToggleToolbar,
    JumpNextUnread,
    /// The AppKey stored here is a placeholder; matching checks for any digit 1–9.
    JumpBufferByNumber,
}

impl KeybindAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::CycleBufferUp      => "Cycle buffer up",
            Self::CycleBufferDown    => "Cycle buffer down",
            Self::ToggleSearch       => "Toggle search",
            Self::ToggleBufferList   => "Toggle buffer list",
            Self::ToggleNicklist     => "Toggle nicklist",
            Self::ToggleToolbar      => "Toggle toolbar",
            Self::JumpNextUnread     => "Jump to next unread",
            Self::JumpBufferByNumber => "Jump to buffer N (+ 1–9)",
        }
    }

    pub const ALL: &'static [Self] = &[
        Self::CycleBufferUp,
        Self::CycleBufferDown,
        Self::ToggleSearch,
        Self::ToggleBufferList,
        Self::ToggleNicklist,
        Self::ToggleToolbar,
        Self::JumpNextUnread,
        Self::JumpBufferByNumber,
    ];
}

pub type Shortcut = (SerdeModifiers, AppKey);

fn default_keybinds() -> HashMap<KeybindAction, Shortcut> {
    use KeybindAction::*;
    use AppKey::*;
    let cmd = SerdeModifiers { command: true, ..Default::default() };
    let alt = SerdeModifiers { alt: true, ..Default::default() };
    [
        (CycleBufferUp,      (cmd,  ArrowUp)),
        (CycleBufferDown,    (cmd,  ArrowDown)),
        (ToggleSearch,       (cmd,  F)),
        (ToggleBufferList,   (cmd,  B)),
        (ToggleNicklist,     (cmd,  N)),
        (ToggleToolbar,      (cmd,  T)),
        (JumpNextUnread,     (cmd,  K)),
        (JumpBufferByNumber, (alt,  Num1)),
    ].into()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct KeybindsMap(pub HashMap<KeybindAction, Shortcut>);

impl Default for KeybindsMap {
    fn default() -> Self {
        Self(default_keybinds())
    }
}

impl KeybindsMap {
    /// Returns true and consumes the event if the given action's shortcut was triggered.
    pub fn consume(&self, i: &mut egui::InputState, action: KeybindAction) -> bool {
        let Some((mods, key)) = self.0.get(&action) else { return false };
        i.consume_key(mods.to_egui(), key.to_egui())
    }

    /// Check if any digit key 1–9 was pressed with the JumpBufferByNumber modifier.
    /// Returns Some(1..=9) if matched, consuming the event.
    pub fn consume_jump_number(&self, i: &mut egui::InputState) -> Option<usize> {
        let Some((mods, _)) = self.0.get(&KeybindAction::JumpBufferByNumber) else { return None };
        if mods.is_empty() { return None; }
        let egui_mods = mods.to_egui();
        let digit_keys = [
            (egui::Key::Num1, 1usize), (egui::Key::Num2, 2), (egui::Key::Num3, 3),
            (egui::Key::Num4, 4),      (egui::Key::Num5, 5), (egui::Key::Num6, 6),
            (egui::Key::Num7, 7),      (egui::Key::Num8, 8), (egui::Key::Num9, 9),
        ];
        for (k, n) in digit_keys {
            if i.consume_key(egui_mods, k) {
                return Some(n);
            }
        }
        None
    }

    /// Human-readable label for a shortcut, e.g. "⌘+↑" or "Ctrl+↑".
    pub fn shortcut_label(&self, action: KeybindAction) -> String {
        match self.0.get(&action) {
            Some((mods, key)) => {
                let m = mods.display();
                let k = if action == KeybindAction::JumpBufferByNumber {
                    "1–9".to_string()
                } else {
                    key.label().to_string()
                };
                if m.is_empty() { k } else { format!("{}+{}", m, k) }
            }
            None => "—".to_string(),
        }
    }
}
