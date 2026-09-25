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

/// Milestone 8 end to end: a plan approved with the CLI, then executed by
/// `keyjutsu run` in Performance mode inside a pseudo-console, with keys
/// mashed through the console input stack.
#[test]
fn run_executes_an_approved_snapshot_in_performance_mode() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("run-e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let plan = serde_json::json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t", "title": "Say hello",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [
            {"id": "hello", "title": "Say hello", "objective": "Print a word.", "kind": "command",
             "shell": {"kind": "pwsh"}, "commands": [{"text": "Write-Output ('run' + '-ok')"}],
             "internal_validation": [{"exit_code": {"equals": 0}}]}
        ]
    });
    let plan_path = dir.join("plan.json");
    std::fs::write(&plan_path, plan.to_string()).unwrap();
    let snap = dir.join("snap.json");
    let approved = std::process::Command::new(env!("CARGO_BIN_EXE_keyjutsu"))
        .args(["plan", "approve", plan_path.to_str().unwrap(), "--out", snap.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(approved.status.success(), "{}", String::from_utf8_lossy(&approved.stderr));

    let pair = native_pty_system()
        .openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_keyjutsu"));
    cmd.args(["run", snap.to_str().unwrap(), "--mode", "performance", "--clean"]);
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
    let screen = Arc::new(Screen { text: Mutex::new(String::new()), changed: Condvar::new() });
    {
        let (screen, writer) = (screen.clone(), writer.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
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

    // Mash the way an operator does: from the start, until something happens.
    let deadline = Instant::now() + TIMEOUT;
    while !screen.plain().contains("run-ok") {
        assert!(Instant::now() < deadline, "the step never ran:\n{:?}", screen.text.lock().unwrap());
        send(b"q");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!screen.plain().contains("qq"), "a mashed key reached the shell:\n{}", screen.plain());

    // Disarm, then leave the shell; the outcome is printed on the way out.
    std::thread::sleep(Duration::from_millis(500));
    send(&win32_key(0x4B, 0x25, 0x0B, 0x1A));
    send(b"exit\r");
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "keyjutsu run did not exit:\n{}", screen.plain());
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(screen.wait_for("Complete: 1 steps."), "no outcome:\n{}", screen.plain());
    assert!(status.success(), "{status:?}");
    assert!(dir.join("snap.checkpoint.json").exists(), "a checkpoint was written");
}

#[test]
fn run_resumes_from_the_checkpoint_it_is_given() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("run-resume");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let plan = serde_json::json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [{"id": "a", "title": "A", "objective": "Look.", "kind": "command",
                   "shell": {"kind": "pwsh"}, "commands": [{"text": "Get-Date"}]}]
    });
    let plan_path = dir.join("plan.json");
    std::fs::write(&plan_path, plan.to_string()).unwrap();
    let snap = dir.join("v2.json");
    let keyjutsu = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_keyjutsu")).args(args).output().unwrap()
    };
    let approved =
        keyjutsu(&["plan", "approve", plan_path.to_str().unwrap(), "--out", snap.to_str().unwrap()]);
    assert!(approved.status.success(), "{}", String::from_utf8_lossy(&approved.stderr));

    // The named checkpoint is read, not the one next to the snapshot.
    let old = dir.join("v1.checkpoint.json");
    let out = keyjutsu(&["run", snap.to_str().unwrap(), "--resume", old.to_str().unwrap()]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("cannot resume from") && err.contains("v1.checkpoint.json"), "{err}");
}
