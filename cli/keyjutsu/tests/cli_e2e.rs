//! The CLI end to end: `keyjutsu.exe` runs in a pseudo-console of its own, as
//! it would in Windows Terminal, and receives keys through the console input
//! stack rather than through any KeyJutsu API.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![cfg(windows)]

use std::io::{Read, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const TIMEOUT: Duration = Duration::from_secs(40);

struct Screen {
    text: Mutex<String>,
    changed: Condvar,
}

impl Screen {
    fn wait_for(&self, needle: &str) -> bool {
        let deadline = Instant::now() + TIMEOUT;
        let mut text = self.text.lock().unwrap();
        while !strip(&text).contains(needle) {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            text = self.changed.wait_timeout(text, deadline - now).unwrap().0;
        }
        true
    }

    fn plain(&self) -> String {
        strip(&self.text.lock().unwrap())
    }
}

fn strip(text: &str) -> String {
    let mut out = String::new();
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
                    if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// A key event in ConPTY's win32-input-mode encoding, which carries exact
/// modifier state: `ESC [ Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`.
fn win32_key(vk: u16, scan: u16, unicode: u16, modifiers: u32) -> Vec<u8> {
    let mut bytes = format!("\x1b[{vk};{scan};{unicode};1;{modifiers};1_").into_bytes();
    bytes.extend(format!("\x1b[{vk};{scan};{unicode};0;{modifiers};1_").into_bytes());
    bytes
}

#[test]
fn perform_types_the_staged_command_and_disarms_through_the_console() {
    let pair = native_pty_system()
        .openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_keyjutsu"));
    cmd.args(["perform", "--clean", "-c", "Write-Output ('cli' + '-ok')"]);
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
    let screen = Arc::new(Screen { text: Mutex::new(String::new()), changed: Condvar::new() });
    {
        let screen = screen.clone();
        let writer = writer.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                // This test is the outer terminal, so it answers ConPTY's
                // cursor query, as Windows Terminal would.
                if chunk.contains("\x1b[6n") {
                    let _ = writer.lock().unwrap().write_all(b"\x1b[1;1R");
                }
                screen.text.lock().unwrap().push_str(&chunk);
                screen.changed.notify_all();
            }
        });
    }
    let send = |bytes: &[u8]| {
        let mut w = writer.lock().unwrap();
        w.write_all(bytes).unwrap();
        w.flush().unwrap();
    };

    assert!(screen.wait_for("PS "), "no prompt:\n{}", screen.plain());
    // Mash keys that do not occur anywhere in the command or its output.
    let command_len = "Write-Output ('cli' + '-ok')".len();
    for _ in 0..=command_len {
        send(b"z");
    }
    assert!(screen.wait_for("cli-ok"), "command never ran:\n{}", screen.plain());
    assert!(!screen.plain().contains("zz"), "a mashed key reached the shell:\n{}", screen.plain());

    // After the last step KeyJutsu holds the keyboard; this is swallowed.
    send(b"exit\r");
    std::thread::sleep(Duration::from_millis(500));
    assert!(child.try_wait().unwrap().is_none(), "keys leaked through after completion");

    // Ctrl+Alt+Shift+K: VK 0x4B, scan 0x25, Ctrl+K is U+000B, and
    // LEFT_ALT | LEFT_CTRL | SHIFT is 0x1A.
    send(&win32_key(0x4B, 0x25, 0x0B, 0x1A));
    send(b"exit\r");

    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "keyjutsu did not exit after disarm:\n{}", screen.plain());
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(screen.wait_for("Step 1: succeeded (exit 0)"), "no summary:\n{}", screen.plain());
    assert!(status.success(), "exit status {status:?}");
}
