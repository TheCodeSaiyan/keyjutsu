//! Running a session with nobody watching: the readiness probe, the tests and
//! anything else that needs to drive a real shell and inspect what happened.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::session::{SessionEvent, SessionSink};

/// A sink that keeps everything, and lets a caller wait for an event.
#[derive(Debug, Default)]
pub struct Collector {
    state: Mutex<Collected>,
    changed: Condvar,
}

#[derive(Debug, Default, Clone)]
pub struct Collected {
    pub output: String,
    pub events: Vec<SessionEvent>,
}

impl Collector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Collected {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Output with escape sequences removed, as a person would read it.
    pub fn plain_output(&self) -> String {
        strip_ansi(&self.snapshot().output)
    }

    /// Wait until `done` holds for what has been collected, or `timeout`
    /// passes. Returns whether it held. Woken by each new output or event,
    /// not by polling.
    pub fn wait_until(&self, timeout: Duration, mut done: impl FnMut(&Collected) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if done(&state) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            state = self.changed.wait_timeout(state, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

impl SessionSink for Collector {
    fn output(&self, text: &str) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).output.push_str(text);
        self.changed.notify_all();
    }

    fn event(&self, event: SessionEvent) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).events.push(event);
        self.changed.notify_all();
    }
}

/// Remove CSI, OSC and two-byte escape sequences.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_cursor_and_title_sequences() {
        let raw = "\x1b[93mecho \x1b[37mhi\x1b[80X\r\n\x1b[?25h\x1b]0;C:\\pwsh.exe\x07ok\x1b]133;A\x1b\\";
        assert_eq!(strip_ansi(raw), "echo hi\r\nok");
    }
}
