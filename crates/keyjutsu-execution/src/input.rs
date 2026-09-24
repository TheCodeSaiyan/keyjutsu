//! What a physical key means while KeyJutsu owns the input line.
//!
//! Classification happens here rather than in either front end so that the
//! desktop and the CLI cannot disagree about it, and so the hard-disarm chord
//! is recognised before anything else looks at the key.

use keyjutsu_terminal::{KeyChord, KeyName};
use serde::{Deserialize, Serialize};

/// The operator's special chords.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct Bindings {
    /// Stops staged input at once and hands the terminal back.
    pub hard_disarm: KeyChord,
    /// Opens the private operator overlay.
    pub overlay: KeyChord,
}

impl Default for Bindings {
    fn default() -> Self {
        // `Esc` is deliberately not a default: shells and full-screen programs
        // use it, and a disarm key that fires by accident ends a performance.
        Self {
            hard_disarm: KeyChord {
                key: KeyName::Char('k'),
                ctrl: true,
                alt: true,
                shift: true,
                meta: false,
            },
            overlay: KeyChord { key: KeyName::Char('k'), ctrl: true, alt: false, shift: true, meta: false },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BindingError {
    #[error("the {0} chord needs Ctrl, Alt or Win, so ordinary typing cannot trigger it")]
    NoModifier(&'static str),
    #[error("the {0} chord cannot be Ctrl+C, which is the real interrupt")]
    Interrupt(&'static str),
    #[error("the disarm and overlay chords must differ")]
    Same,
}

impl Bindings {
    pub fn validate(&self) -> Result<(), BindingError> {
        for (name, chord) in [("disarm", &self.hard_disarm), ("overlay", &self.overlay)] {
            if !(chord.ctrl || chord.alt || chord.meta) {
                return Err(BindingError::NoModifier(name));
            }
            if is_interrupt(chord) {
                return Err(BindingError::Interrupt(name));
            }
        }
        if self.hard_disarm.matches(&self.overlay) {
            return Err(BindingError::Same);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum KeyClass {
    HardDisarm,
    Overlay,
    /// Ctrl+C: always a genuine interrupt, never staged typing.
    Interrupt,
    /// Esc keeps its normal terminal meaning.
    Escape,
    Enter,
    /// An ordinary printable key. Its value is ignored for staged text.
    Advance,
    /// Navigation, editing and function keys: no staged effect.
    Other,
}

fn is_interrupt(chord: &KeyChord) -> bool {
    matches!(chord.key, KeyName::Char('c' | 'C')) && chord.ctrl && !chord.alt && !chord.meta && !chord.shift
}

pub fn classify(chord: &KeyChord, bindings: &Bindings) -> KeyClass {
    if chord.matches(&bindings.hard_disarm) {
        return KeyClass::HardDisarm;
    }
    if chord.matches(&bindings.overlay) {
        return KeyClass::Overlay;
    }
    if is_interrupt(chord) {
        return KeyClass::Interrupt;
    }
    match chord.key {
        KeyName::Escape => KeyClass::Escape,
        KeyName::Enter => KeyClass::Enter,
        // Ctrl+Alt arrives with AltGr characters on European layouts, which
        // are ordinary typing.
        KeyName::Char(_) if !chord.meta && (!chord.ctrl || chord.alt) => KeyClass::Advance,
        _ => KeyClass::Other,
    }
}

/// A classified key, with the bytes it would send if passed to the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInput {
    pub class: KeyClass,
    pub bytes: Option<Vec<u8>>,
}

impl KeyInput {
    pub fn from_chord(chord: &KeyChord, bindings: &Bindings) -> Self {
        Self { class: classify(chord, bindings), bytes: chord.encode_vt() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(c: char, ctrl: bool, alt: bool, shift: bool) -> KeyChord {
        KeyChord { key: KeyName::Char(c), ctrl, alt, shift, meta: false }
    }

    #[test]
    fn the_hard_disarm_chord_wins_over_every_other_meaning() {
        let b = Bindings::default();
        assert_eq!(classify(&chord('K', true, true, true), &b), KeyClass::HardDisarm);
        assert_eq!(classify(&chord('k', true, true, true), &b), KeyClass::HardDisarm);
        assert_eq!(classify(&chord('K', true, false, true), &b), KeyClass::Overlay);
    }

    #[test]
    fn escape_is_never_a_disarm() {
        let b = Bindings::default();
        assert_eq!(classify(&KeyChord::plain(KeyName::Escape), &b), KeyClass::Escape);
    }

    #[test]
    fn ctrl_c_is_an_interrupt_and_ordinary_letters_advance() {
        let b = Bindings::default();
        assert_eq!(classify(&chord('c', true, false, false), &b), KeyClass::Interrupt);
        for c in "asdfghjkl;' 1/".chars() {
            assert_eq!(classify(&KeyChord::char(c), &b), KeyClass::Advance, "{c:?}");
        }
        assert_eq!(classify(&chord('A', false, false, true), &b), KeyClass::Advance);
        assert_eq!(classify(&chord('@', true, true, false), &b), KeyClass::Advance, "AltGr");
        assert_eq!(classify(&chord('v', true, false, false), &b), KeyClass::Other);
        assert_eq!(classify(&KeyChord::plain(KeyName::Enter), &b), KeyClass::Enter);
        assert_eq!(classify(&KeyChord::plain(KeyName::Backspace), &b), KeyClass::Other);
        assert_eq!(classify(&KeyChord::plain(KeyName::Up), &b), KeyClass::Other);
    }

    #[test]
    fn bindings_that_ordinary_typing_could_trigger_are_rejected() {
        assert_eq!(Bindings::default().validate(), Ok(()));
        let plain = Bindings { hard_disarm: chord('k', false, false, true), ..Bindings::default() };
        assert_eq!(plain.validate(), Err(BindingError::NoModifier("disarm")));
        let esc = Bindings { hard_disarm: KeyChord::plain(KeyName::Escape), ..Bindings::default() };
        assert_eq!(esc.validate(), Err(BindingError::NoModifier("disarm")));
        let interrupt = Bindings { overlay: chord('c', true, false, false), ..Bindings::default() };
        assert_eq!(interrupt.validate(), Err(BindingError::Interrupt("overlay")));
        let same = Bindings { overlay: Bindings::default().hard_disarm, ..Bindings::default() };
        assert_eq!(same.validate(), Err(BindingError::Same));
    }
}
