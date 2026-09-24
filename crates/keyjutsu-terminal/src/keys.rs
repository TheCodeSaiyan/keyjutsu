//! Physical key chords, as the desktop and CLI front ends report them, and
//! their encoding as terminal input.
//!
//! Front ends do not decide what a key means. They describe the key that was
//! pressed; the execution engine classifies it, and this module turns it into
//! the bytes a shell expects when it is passed through for real.

use serde::{Deserialize, Serialize};

/// A key, independent of any front end's event type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum KeyName {
    /// A key that produces a character. The character is what the keyboard
    /// layout produced, so Shift+k arrives as `K`.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// Function keys F1 to F24.
    F(u8),
    /// Anything else: modifiers on their own, media keys, IME composition.
    Other,
}

/// A key together with the modifiers held when it was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct KeyChord {
    pub key: KeyName,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub meta: bool,
}

impl KeyChord {
    pub fn plain(key: KeyName) -> Self {
        Self { key, ctrl: false, alt: false, shift: false, meta: false }
    }

    pub fn char(c: char) -> Self {
        Self::plain(KeyName::Char(c))
    }

    /// Parse a binding such as `Ctrl+Alt+Shift+K`. Modifier names are
    /// case-insensitive; the final segment is a single character or a key name.
    pub fn parse_binding(text: &str) -> Option<Self> {
        let mut chord = Self::plain(KeyName::Other);
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (last, modifiers) = parts.split_last()?;
        for m in modifiers {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => chord.ctrl = true,
                "alt" => chord.alt = true,
                "shift" => chord.shift = true,
                "meta" | "win" | "super" => chord.meta = true,
                _ => return None,
            }
        }
        let mut chars = last.chars();
        chord.key = match (chars.next(), chars.next()) {
            (Some(c), None) => KeyName::Char(c),
            _ => match last.to_ascii_lowercase().as_str() {
                "enter" => KeyName::Enter,
                "tab" => KeyName::Tab,
                "backspace" => KeyName::Backspace,
                "esc" | "escape" => KeyName::Escape,
                "space" => KeyName::Char(' '),
                f if f.starts_with('f') => KeyName::F(f[1..].parse().ok()?),
                _ => return None,
            },
        };
        Some(chord)
    }

    /// Whether this key press is the given binding. Modifiers must match
    /// exactly, so Ctrl+Shift+K is not Ctrl+Alt+Shift+K; letters match
    /// regardless of case, because Shift changes the character a layout
    /// reports.
    pub fn matches(&self, binding: &KeyChord) -> bool {
        let same_key = match (self.key, binding.key) {
            (KeyName::Char(a), KeyName::Char(b)) => a.to_lowercase().eq(b.to_lowercase()),
            (a, b) => a == b,
        };
        same_key
            && self.ctrl == binding.ctrl
            && self.alt == binding.alt
            && self.shift == binding.shift
            && self.meta == binding.meta
    }

    /// The bytes a VT terminal sends for this key, or `None` for keys that
    /// send nothing.
    pub fn encode_vt(&self) -> Option<Vec<u8>> {
        let modifier = 1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl);
        let csi = |final_byte: char| -> Vec<u8> {
            if modifier > 1 {
                format!("\x1b[1;{modifier}{final_byte}").into_bytes()
            } else {
                format!("\x1b[{final_byte}").into_bytes()
            }
        };
        let tilde = |code: u8| -> Vec<u8> {
            if modifier > 1 {
                format!("\x1b[{code};{modifier}~").into_bytes()
            } else {
                format!("\x1b[{code}~").into_bytes()
            }
        };
        let bytes = match self.key {
            KeyName::Char(c) => return Some(self.encode_char(c)),
            KeyName::Enter => with_alt(self.alt, b"\r"),
            KeyName::Tab if self.shift => b"\x1b[Z".to_vec(),
            KeyName::Tab => b"\t".to_vec(),
            KeyName::Backspace if self.ctrl => b"\x08".to_vec(),
            KeyName::Backspace => with_alt(self.alt, b"\x7f"),
            KeyName::Escape => b"\x1b".to_vec(),
            KeyName::Up => csi('A'),
            KeyName::Down => csi('B'),
            KeyName::Right => csi('C'),
            KeyName::Left => csi('D'),
            KeyName::Home => csi('H'),
            KeyName::End => csi('F'),
            KeyName::Insert => tilde(2),
            KeyName::Delete => tilde(3),
            KeyName::PageUp => tilde(5),
            KeyName::PageDown => tilde(6),
            KeyName::F(n @ 1..=4) if modifier == 1 => vec![0x1b, b'O', b'P' + (n - 1)],
            KeyName::F(n @ 1..=4) => format!("\x1b[1;{modifier}{}", (b'P' + (n - 1)) as char).into_bytes(),
            KeyName::F(n) => {
                const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
                tilde(*CODES.get(usize::from(n).checked_sub(5)?)?)
            }
            KeyName::Other => return None,
        };
        Some(bytes)
    }

    fn encode_char(&self, c: char) -> Vec<u8> {
        // Ctrl+Alt with a non-letter is AltGr on European layouts: the
        // character has already been produced and is sent as it is.
        let is_altgr = self.alt && !c.is_ascii_alphabetic();
        if self.ctrl
            && !is_altgr
            && let Some(control) = control_byte(c)
        {
            return with_alt(self.alt, &[control]);
        }
        let mut buf = [0u8; 4];
        let text = c.encode_utf8(&mut buf).as_bytes();
        with_alt(self.alt && !self.ctrl, text)
    }
}

fn control_byte(c: char) -> Option<u8> {
    match c.to_ascii_lowercase() {
        l @ 'a'..='z' => Some(l as u8 & 0x1f),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        _ => None,
    }
}

fn with_alt(alt: bool, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 1);
    if alt {
        out.push(0x1b);
    }
    out.extend_from_slice(bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(key: KeyName, ctrl: bool, alt: bool, shift: bool) -> KeyChord {
        KeyChord { key, ctrl, alt, shift, meta: false }
    }

    #[test]
    fn parses_the_default_bindings() {
        let disarm = KeyChord::parse_binding("Ctrl+Alt+Shift+K").unwrap();
        assert_eq!(disarm, chord(KeyName::Char('K'), true, true, true));
        let overlay = KeyChord::parse_binding("ctrl + shift + k").unwrap();
        assert_eq!(overlay, chord(KeyName::Char('k'), true, false, true));
        assert_eq!(KeyChord::parse_binding("Ctrl+F12").unwrap().key, KeyName::F(12));
        assert!(KeyChord::parse_binding("Hyper+K").is_none());
        assert!(KeyChord::parse_binding("Ctrl+Nonsense").is_none());
    }

    #[test]
    fn matching_needs_exact_modifiers_but_ignores_letter_case() {
        let disarm = KeyChord::parse_binding("Ctrl+Alt+Shift+K").unwrap();
        assert!(chord(KeyName::Char('K'), true, true, true).matches(&disarm));
        assert!(chord(KeyName::Char('k'), true, true, true).matches(&disarm));
        assert!(!chord(KeyName::Char('K'), true, false, true).matches(&disarm));
        assert!(!chord(KeyName::Char('K'), false, false, true).matches(&disarm));
    }

    #[test]
    fn encodes_printable_and_control_characters() {
        assert_eq!(KeyChord::char('a').encode_vt().unwrap(), b"a");
        assert_eq!(KeyChord::char('é').encode_vt().unwrap(), "é".as_bytes());
        assert_eq!(chord(KeyName::Char('c'), true, false, false).encode_vt().unwrap(), b"\x03");
        assert_eq!(chord(KeyName::Char('C'), true, false, true).encode_vt().unwrap(), b"\x03");
        assert_eq!(chord(KeyName::Char('x'), false, true, false).encode_vt().unwrap(), b"\x1bx");
        assert_eq!(chord(KeyName::Char('b'), true, true, false).encode_vt().unwrap(), b"\x1b\x02");
    }

    #[test]
    fn altgr_characters_are_sent_as_produced() {
        // AltGr+2 on a German layout produces '@' with Ctrl and Alt held.
        assert_eq!(chord(KeyName::Char('²'), true, true, false).encode_vt().unwrap(), "²".as_bytes());
        assert_eq!(chord(KeyName::Char('{'), true, true, false).encode_vt().unwrap(), b"{");
    }

    #[test]
    fn encodes_navigation_and_editing_keys() {
        assert_eq!(KeyChord::plain(KeyName::Enter).encode_vt().unwrap(), b"\r");
        assert_eq!(KeyChord::plain(KeyName::Backspace).encode_vt().unwrap(), b"\x7f");
        assert_eq!(KeyChord::plain(KeyName::Escape).encode_vt().unwrap(), b"\x1b");
        assert_eq!(KeyChord::plain(KeyName::Up).encode_vt().unwrap(), b"\x1b[A");
        assert_eq!(chord(KeyName::Left, true, false, false).encode_vt().unwrap(), b"\x1b[1;5D");
        assert_eq!(KeyChord::plain(KeyName::Delete).encode_vt().unwrap(), b"\x1b[3~");
        assert_eq!(chord(KeyName::Tab, false, false, true).encode_vt().unwrap(), b"\x1b[Z");
        assert_eq!(KeyChord::plain(KeyName::F(1)).encode_vt().unwrap(), b"\x1bOP");
        assert_eq!(KeyChord::plain(KeyName::F(5)).encode_vt().unwrap(), b"\x1b[15~");
        assert_eq!(KeyChord::plain(KeyName::F(12)).encode_vt().unwrap(), b"\x1b[24~");
        assert!(KeyChord::plain(KeyName::F(13)).encode_vt().is_none());
        assert!(KeyChord::plain(KeyName::Other).encode_vt().is_none());
    }

    #[test]
    fn serialises_in_the_shape_the_front_ends_send() {
        let c: KeyChord = serde_json::from_str(r#"{"key":{"char":"k"},"ctrl":true}"#).unwrap();
        assert_eq!(c, chord(KeyName::Char('k'), true, false, false));
        let e: KeyChord = serde_json::from_str(r#"{"key":"enter"}"#).unwrap();
        assert_eq!(e, KeyChord::plain(KeyName::Enter));
    }
}
