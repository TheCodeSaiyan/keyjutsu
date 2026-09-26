//! Drives the installed `keyjutsu demo` in Performance mode:
//! mashes keys in a pseudo-console of its own, as someone at the keyboard
//! would, until the demo has typed and run its last command, then disarms and
//! leaves. For a clean machine such as Windows Sandbox.
//!
//! `demo_trial PATH\TO\keyjutsu.exe`

#![allow(clippy::unwrap_used, clippy::print_stdout)] // A trial program, not a library.

use std::io::{Read, Write};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

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

/// A key in ConPTY's win32-input-mode encoding, which carries the modifiers.
fn win32_key(vk: u16, scan: u16, unicode: u16, modifiers: u32) -> Vec<u8> {
    let mut bytes = format!("\x1b[{vk};{scan};{unicode};1;{modifiers};1_").into_bytes();
    bytes.extend(format!("\x1b[{vk};{scan};{unicode};0;{modifiers};1_").into_bytes());
    bytes
}

fn main() -> ExitCode {
    let Some(exe) = std::env::args().nth(1) else {
        println!("usage: demo_trial PATH\\TO\\keyjutsu.exe");
        return ExitCode::from(2);
    };
    let pair = native_pty_system()
        .openpty(PtySize { rows: 40, cols: 160, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let mut cmd = CommandBuilder::new(&exe);
    cmd.args(["demo", "--clean"]);
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();
    // One thread writes, fed by a channel, so neither the reader nor the
    // loop below ever waits on a write. With a shared locked writer, a write
    // that blocked (the console busy, not reading input) held the lock while
    // the reader waited on it to answer a cursor query, nothing drained the
    // output, and the whole trial hung past its own deadline.
    let (to_pty, keys) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        for bytes in keys {
            if writer.write_all(&bytes).and_then(|()| writer.flush()).is_err() {
                break;
            }
        }
    });
    let screen = Arc::new(Mutex::new(String::new()));
    {
        let (screen, to_pty) = (screen.clone(), to_pty.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                if chunk.contains("\x1b[6n") {
                    let _ = to_pty.send(b"\x1b[1;1R".to_vec());
                }
                screen.lock().unwrap().push_str(&chunk);
            }
        });
    }
    let send = |b: &[u8]| {
        let _ = to_pty.send(b.to_vec());
    };
    let plain = || strip(&screen.lock().unwrap());

    // Mash until the last command has been typed and the prompt is back after
    // it. Its output is not the sign: Windows Sandbox lists no volumes, so
    // the table there is empty. Whether each step succeeded is read from
    // KeyJutsu's own summary on the way out.
    let last_command_done = |text: &str| text.rfind("-AutoSize").is_some_and(|at| text[at..].contains("PS "));
    let deadline = Instant::now() + Duration::from_secs(240);
    let mut keys = 0u32;
    while !last_command_done(&plain()) {
        if Instant::now() > deadline {
            let text = plain();
            let tail: String = text.chars().rev().take(4000).collect::<Vec<_>>().into_iter().rev().collect();
            println!("FAIL: the demo did not finish after {keys} keys. End of the screen:\n{tail}");
            let _ = child.kill();
            return ExitCode::FAILURE;
        }
        send(b"q");
        keys += 1;
        std::thread::sleep(Duration::from_millis(15));
    }
    let typed_for_real = !plain().contains("qqq");
    println!("demo finished after {keys} mashed keys; no mashed key reached the shell: {typed_for_real}");

    std::thread::sleep(Duration::from_millis(800));
    send(&win32_key(0x4B, 0x25, 0x0B, 0x1A)); // Ctrl+Alt+Shift+K: disarm
    send(b"exit\r");
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() > deadline {
            println!("FAIL: keyjutsu demo did not exit");
            return ExitCode::FAILURE;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    println!("keyjutsu demo exited: {status:?}");
    let text = plain();
    for line in ["OsName", "PSVersion", "DriveLetter FileSystemLabel FileSystem"] {
        println!("  output shows {line}: {}", text.contains(line));
    }
    let summary = [
        " 1. Describe this computer: succeeded (exit 0)",
        " 2. Show the PowerShell version: succeeded (exit 0)",
        " 3. List the volumes: succeeded (exit 0)",
    ];
    let mut all_succeeded = true;
    for line in summary {
        let seen = text.contains(line);
        all_succeeded &= seen;
        println!("  summary says{line}: {seen}");
    }
    if status.success() && typed_for_real && all_succeeded && text.contains("PSVersion") {
        println!("PASS demo: typed by mashing, ran for real, disarmed and exited");
        ExitCode::SUCCESS
    } else {
        println!("FAIL demo");
        ExitCode::FAILURE
    }
}
