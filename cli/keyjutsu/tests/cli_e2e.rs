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

/// `keyjutsu` in a pseudo-console of its own, answering ConPTY's cursor
/// query as a terminal would. Returns the child, what it drew, and a writer.
#[allow(clippy::type_complexity)]
fn launch(
    args: &[&str],
) -> (Box<dyn portable_pty::Child + Send + Sync>, Arc<Screen>, Arc<Mutex<Box<dyn Write + Send>>>) {
    let pair = native_pty_system()
        .openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_keyjutsu"));
    cmd.args(args);
    let child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let writer = Arc::new(Mutex::new(pair.master.take_writer().unwrap()));
    let screen = Arc::new(Screen { text: Mutex::new(String::new()), changed: Condvar::new() });
    {
        let (screen, writer) = (screen.clone(), writer.clone());
        std::thread::spawn(move || {
            let _master = pair.master;
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
    (child, screen, writer)
}

#[test]
fn run_asks_for_a_credential_in_the_shells_masked_prompt() {
    const SECRET: &str = "cli-S3cret-99";
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("run-credential");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let plan = serde_json::json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [
            {"id": "ask", "title": "Token", "objective": "Get a token.", "kind": "credential",
             "shell": {"kind": "pwsh"}, "execution_mode": "user_input",
             "credential": {"variable": "KJ_CLI_TOKEN", "prompt": "Token for the CLI test", "kind": "secret"}},
            {"id": "use", "title": "Use it", "objective": "Use the token.", "kind": "command",
             "shell": {"kind": "pwsh"}, "depends_on": ["ask"],
             "commands": [{"text": "'length ' + [System.Net.NetworkCredential]::new('', $KJ_CLI_TOKEN).Password.Length"}]}
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

    let (mut child, screen, writer) =
        launch(&["run", snap.to_str().unwrap(), "--mode", "performance", "--clean"]);
    let send = |bytes: &[u8]| {
        let mut w = writer.lock().unwrap();
        w.write_all(bytes).unwrap();
        w.flush().unwrap();
    };
    let raw = || screen.text.lock().unwrap().clone();

    // The operator mashes until KeyJutsu says a credential is needed; the
    // mashing starts nothing.
    let deadline = Instant::now() + TIMEOUT;
    while !raw().contains("Credential required: Token for the CLI test") {
        assert!(Instant::now() < deadline, "no credential notice:\n{:?}", raw());
        send(b"q");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(!screen.plain().contains("Token for the CLI test:"), "mashing opened the prompt");

    // Enter opens the shell's masked prompt; the secret goes in for real.
    send(b"\r");
    assert!(screen.wait_for("Token for the CLI test: "), "no prompt:\n{}", screen.plain());
    send(SECRET.as_bytes());
    send(b"\r");

    // Back to mashing for the step that uses it.
    let deadline = Instant::now() + TIMEOUT;
    while !screen.plain().contains(&format!("length {}", SECRET.len())) {
        assert!(Instant::now() < deadline, "the token was not used:\n{}", screen.plain());
        send(b"q");
        std::thread::sleep(Duration::from_millis(20));
    }
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
    assert!(screen.wait_for("Complete: 2 steps."), "no outcome:\n{}", screen.plain());
    assert!(status.success(), "{status:?}");

    // The secret never reached the console, the checkpoint or the snapshot.
    assert!(!raw().contains(SECRET), "the secret was drawn");
    assert!(!std::fs::read_to_string(dir.join("snap.checkpoint.json")).unwrap().contains(SECRET));
    assert!(!std::fs::read_to_string(&snap).unwrap().contains(SECRET));
}

#[test]
fn a_failed_run_is_recovered_only_when_the_operator_confirms() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("run-recover");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("settings.txt");
    std::fs::write(&target, "original").unwrap();
    let fwd = |p: &std::path::Path| p.display().to_string().replace('\\', "/");
    let plan = serde_json::json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [
            {"id": "edit", "title": "Edit", "objective": "Change the setting.", "kind": "command",
             "shell": {"kind": "pwsh"}, "commands": [{"text": format!("Set-Content -LiteralPath {} -Value changed", fwd(&target))}],
             "reversibility": {"level": "full"},
             "recovery": {"strategy": "restore_captured_state", "capture": [{"kind": "file", "target": fwd(&target)}]}},
            {"id": "verify", "title": "Verify", "objective": "Fails on purpose.", "kind": "validation",
             "shell": {"kind": "pwsh"}, "depends_on": ["edit"], "commands": [{"text": "Get-Date"}],
             "internal_validation": [{"path_exists": {"path": fwd(&dir.join("never.txt"))}}]}
        ]
    });
    let plan_path = dir.join("plan.json");
    std::fs::write(&plan_path, plan.to_string()).unwrap();
    let snap = dir.join("snap.json");
    let keyjutsu = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_keyjutsu")).args(args).output().unwrap()
    };
    let approved =
        keyjutsu(&["plan", "approve", plan_path.to_str().unwrap(), "--out", snap.to_str().unwrap()]);
    assert!(approved.status.success(), "{}", String::from_utf8_lossy(&approved.stderr));

    let (mut child, screen, writer) = launch(&["run", snap.to_str().unwrap(), "--mode", "direct", "--clean"]);
    let deadline = Instant::now() + TIMEOUT;
    while std::fs::read_to_string(dir.join("snap.checkpoint.json"))
        .map_or(true, |t| !t.contains("\"succeeded\": false"))
    {
        assert!(Instant::now() < deadline, "the run never failed:\n{}", screen.plain());
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(500));
    writer.lock().unwrap().write_all(b"exit\r").unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "keyjutsu run did not exit:\n{:?}", screen.text.lock().unwrap());
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(screen.wait_for("Nothing has been rolled back"), "{}", screen.plain());
    assert_eq!(std::fs::read_to_string(&target).unwrap().trim(), "changed");

    // Reviewing changes nothing.
    let review = keyjutsu(&["recover", snap.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&review.stdout);
    assert!(review.status.success(), "{text}");
    assert!(text.contains("restore") && text.contains("--confirm"), "{text}");
    assert_eq!(std::fs::read_to_string(&target).unwrap().trim(), "changed");

    // Confirming restores it.
    let done = keyjutsu(&["recover", snap.to_str().unwrap(), "--confirm"]);
    let text = String::from_utf8_lossy(&done.stdout);
    assert!(done.status.success(), "{text}{}", String::from_utf8_lossy(&done.stderr));
    assert!(text.contains("Recovered."), "{text}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
}
