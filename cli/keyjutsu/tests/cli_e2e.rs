//! The CLI end to end: `keyjutsu.exe` runs in a pseudo-console of its own, as
//! it would in Windows Terminal, and receives keys through the console input
//! stack rather than through any KeyJutsu API.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Tests start programs directly; no window matters here.

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

    /// As `wait_for`, in the raw stream, where window titles are too.
    fn wait_for_raw(&self, needle: &str) -> bool {
        let deadline = Instant::now() + TIMEOUT;
        let mut text = self.text.lock().unwrap();
        while !text.contains(needle) {
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

/// Where a run starts unless a test says otherwise: outside any Git
/// repository. A run records the repositories it works in, so one started in
/// this repository ran git on it, and a run stopped mid-way could leave
/// .git/index.lock behind, blocking the next commit.
fn outside_any_repository() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("keyjutsu-cli-e2e");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A run lock of its own for each KeyJutsu started here: the tests run side
/// by side, and must not wait on each other or on a real run.
fn own_lock() -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(r"Local\keyjutsu-cli-test-{}-{n}", std::process::id())
}

/// The CLI with the tests' own encrypted store, where the plans they approve
/// are recorded and their runs checked against, apart from the operator's.
fn test_command() -> std::process::Command {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_keyjutsu"));
    c.env("KEYJUTSU_STORE", std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-store"))
        .env("KEYJUTSU_RUN_LOCK", own_lock());
    c
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

/// End to end: a plan approved with the CLI, then executed by
/// `keyjutsu run` in Performance mode inside a pseudo-console, with keys
/// mashed through the console input stack, and recorded (ADR 0021): the
/// recording comes back out of the history as an asciicast with the step's
/// markers and what it printed.
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
    let approved = test_command()
        .args(["plan", "approve", plan_path.to_str().unwrap(), "--out", snap.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(approved.status.success(), "{}", String::from_utf8_lossy(&approved.stderr));

    let pair = native_pty_system()
        .openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_keyjutsu"));
    cmd.env("KEYJUTSU_STORE", std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-store"));
    cmd.env("KEYJUTSU_RUN_LOCK", own_lock());
    cmd.cwd(outside_any_repository());
    cmd.args(["run", snap.to_str().unwrap(), "--mode", "performance", "--clean", "--record"]);
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
    assert!(screen.wait_for("Complete: 1 step."), "no outcome:\n{}", screen.plain());
    assert!(status.success(), "{status:?}");
    assert!(dir.join("snap.checkpoint.json").exists(), "a checkpoint was written");
    assert!(screen.plain().contains("Recorded as session"), "{}", screen.plain());
    let listed = test_command()
        .args(["history", "list"])
        .env("KEYJUTSU_STORE", std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-store"))
        .env("KEYJUTSU_RUN_LOCK", own_lock())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&listed.stdout);
    assert!(
        text.contains("complete") && text.contains("Say hello"),
        "the session is in the history:\n{text}"
    );

    let shown = screen.plain();
    let id = shown
        .split("Recorded as session ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no session id in:\n{shown}"));
    let cast = dir.join("run.cast");
    let exported = test_command()
        .args(["history", "export", id, "--step", "hello", "--out", cast.to_str().unwrap()])
        .env("KEYJUTSU_STORE", std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-store"))
        .env("KEYJUTSU_RUN_LOCK", own_lock())
        .output()
        .unwrap();
    assert!(exported.status.success(), "{}", String::from_utf8_lossy(&exported.stderr));
    let recording = std::fs::read_to_string(&cast).unwrap();
    assert!(recording.starts_with("{\"") && recording.contains("\"version\":2"), "{recording}");
    assert!(recording.contains("\"start:hello\"") && recording.contains("\"end:hello:ok\""), "{recording}");
    assert!(recording.contains("run-ok"), "what the step printed is in it:\n{recording}");
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
    let keyjutsu = |args: &[&str]| test_command().args(args).output().unwrap();
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
    launch_in(args, None)
}

#[allow(clippy::type_complexity)]
fn launch_in(
    args: &[&str],
    cwd: Option<&std::path::Path>,
) -> (Box<dyn portable_pty::Child + Send + Sync>, Arc<Screen>, Arc<Mutex<Box<dyn Write + Send>>>) {
    let pair = native_pty_system()
        .openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_keyjutsu"));
    cmd.args(args);
    cmd.cwd(cwd.map(std::path::Path::to_path_buf).unwrap_or_else(outside_any_repository));
    // Sessions these tests run go to a history of their own, not the operator's.
    cmd.env("KEYJUTSU_STORE", std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-store"));
    cmd.env("KEYJUTSU_RUN_LOCK", own_lock());
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
    let approved = test_command()
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
    let keyjutsu = |args: &[&str]| test_command().args(args).output().unwrap();
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

#[test]
fn run_tells_its_git_changes_from_yours_and_shows_its_diff() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("run-git");
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["-c", "user.name=T", "-c", "user.email=t@example.invalid", "-c", "core.autocrlf=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("b.txt"), "b\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "start"]);
    std::fs::write(repo.join("u.txt"), "mine\n").unwrap();

    let plan = serde_json::json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [{"id": "edit", "title": "Edit", "objective": "Change b.", "kind": "command",
                   "shell": {"kind": "pwsh"}, "commands": [{"text": "Add-Content -LiteralPath b.txt -Value keyjutsu-b"}]}]
    });
    let plan_path = dir.join("plan.json");
    std::fs::write(&plan_path, plan.to_string()).unwrap();
    let snap = dir.join("snap.json");
    let keyjutsu = |args: &[&str]| test_command().args(args).current_dir(&repo).output().unwrap();
    let approved =
        keyjutsu(&["plan", "approve", plan_path.to_str().unwrap(), "--out", snap.to_str().unwrap()]);
    assert!(approved.status.success(), "{}", String::from_utf8_lossy(&approved.stderr));

    let (mut child, screen, writer) =
        launch_in(&["run", snap.to_str().unwrap(), "--mode", "direct", "--clean"], Some(&repo));
    let deadline = Instant::now() + TIMEOUT;
    while std::fs::read_to_string(repo.join("b.txt")).is_ok_and(|t| !t.contains("keyjutsu-b")) {
        assert!(Instant::now() < deadline, "the step never ran:\n{}", screen.plain());
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(800));
    writer.lock().unwrap().write_all(&win32_key(0x4B, 0x25, 0x0B, 0x1A)).unwrap();
    writer.lock().unwrap().write_all(b"exit\r").unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "keyjutsu run did not exit:\n{}", screen.plain());
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(screen.wait_for("Your own changes, untouched: u.txt"), "{}", screen.plain());
    assert!(screen.plain().contains("Changed by KeyJutsu:"), "{}", screen.plain());

    let diff = keyjutsu(&["git", "diff", snap.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&diff.stdout);
    assert!(diff.status.success(), "{text}{}", String::from_utf8_lossy(&diff.stderr));
    assert!(text.contains("+keyjutsu-b"), "{text}");
    assert!(!text.contains("+mine"), "your untracked file is not KeyJutsu's:\n{text}");
}

/// A snapshot with one critical step (deleting `victim`), sealed at `at`.
fn critical_snapshot(dir: &std::path::Path, victim: &std::path::Path, at: &str) -> std::path::PathBuf {
    use keyjutsu_core::plan::{ApprovalBook, ValidPlan, parse_plan, seal};
    use keyjutsu_core::validation::{Options, validate};
    let fwd = victim.display().to_string().replace('\\', "/");
    let draft = parse_plan(
        &serde_json::json!({
            "schema_version": "1.0", "plan_id": "p", "task_id": "t",
            "target": {"id": "local", "kind": "local_windows"},
            "agent": {"name": "codex", "version": "1"},
            "steps": [{"id": "wipe", "title": "Remove the victim", "objective": "Delete it.", "kind": "command",
                       "shell": {"kind": "pwsh"}, "commands": [{"text": format!("Remove-Item -Recurse -Force -LiteralPath {fwd}")}]}]
        })
        .to_string(),
    )
    .unwrap();
    let report = validate(&draft, Options { dry_run: false, ..Options::default() });
    let v = ValidPlan::revalidate(report.record_in(draft.plan(), at), false).unwrap();
    let mut book = ApprovalBook::new();
    assert_eq!(book.approve_all_except_critical(&v, at), ["wipe"]);
    book.approve(&v, "wipe", at, Some("REMOVE THE VICTIM")).unwrap();
    let snap = seal(&v, &book, None, at).unwrap();
    // Recorded as `keyjutsu plan approve` records it, in the tests' store.
    let store = keyjutsu_core::store::Store::open(
        &std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-store"),
    )
    .unwrap();
    keyjutsu_core::approvals::record_approval(&store, &snap).unwrap();
    let path = dir.join(format!("snap-{}.json", at.replace(':', "")));
    std::fs::write(&path, snap.to_json()).unwrap();
    path
}

fn wait_exit(
    child: &mut Box<dyn portable_pty::Child + Send + Sync>,
    screen: &Screen,
) -> portable_pty::ExitStatus {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "keyjutsu run did not exit:\n{}", screen.plain());
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn an_old_approval_of_a_critical_step_is_confirmed_again_just_before_it_runs_in_the_cli() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("run-stale-critical");
    let _ = std::fs::remove_dir_all(&dir);
    let victim = dir.join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("keep.txt"), "x").unwrap();
    let stale = critical_snapshot(&dir, &victim, "2026-09-24T00:00:00Z");
    let args = |snap: &std::path::Path| {
        vec![
            "run".to_owned(),
            snap.display().to_string(),
            "--mode".into(),
            "direct".into(),
            "--clean".into(),
            "--ephemeral".into(),
        ]
    };

    // Approved yesterday: asked again in the run's console, just before the
    // step, and a wrong answer runs nothing. Keys typed while it asks go to
    // the answer, not the shell.
    let a = args(&stale);
    let (mut child, screen, writer) = launch_in(&a.iter().map(String::as_str).collect::<Vec<_>>(), None);
    assert!(screen.wait_for("Type REMOVE THE VICTIM to let it run"), "{}", screen.plain());
    writer.lock().unwrap().write_all(b"yes\r").unwrap();
    assert!(screen.wait_for("Not confirmed: it will not run"), "{}", screen.plain());
    assert!(victim.join("keep.txt").exists());
    // The shell is handed back, and says so in the window's title.
    assert!(screen.wait_for_raw("the run has ended. Type exit to leave."), "{}", screen.plain());
    writer.lock().unwrap().write_all(b"exit\r").unwrap();
    let status = wait_exit(&mut child, &screen);
    assert!(!status.success());
    assert!(screen.wait_for("was not confirmed"), "{}", screen.plain());
    assert!(victim.join("keep.txt").exists());

    // The right phrase lets it run.
    let (mut child, screen, writer) = launch_in(&a.iter().map(String::as_str).collect::<Vec<_>>(), None);
    assert!(screen.wait_for("Type REMOVE THE VICTIM to let it run"), "{}", screen.plain());
    writer.lock().unwrap().write_all(b"REMOVE THE VICTIM\r").unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while victim.exists() {
        assert!(Instant::now() < deadline, "the step never ran:\n{}", screen.plain());
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(Duration::from_millis(500));
    writer.lock().unwrap().write_all(&win32_key(0x4B, 0x25, 0x0B, 0x1A)).unwrap();
    writer.lock().unwrap().write_all(b"exit\r").unwrap();
    assert!(wait_exit(&mut child, &screen).success(), "{}", screen.plain());

    // Approved just now: the phrase typed at approval stands.
    std::fs::create_dir_all(&victim).unwrap();
    let fresh = critical_snapshot(&dir, &victim, &keyjutsu_core::fingerprint::now_rfc3339());
    let a = args(&fresh);
    let (mut child, screen, writer) = launch_in(&a.iter().map(String::as_str).collect::<Vec<_>>(), None);
    let deadline = Instant::now() + TIMEOUT;
    while victim.exists() {
        assert!(Instant::now() < deadline, "a fresh approval should run without asking:\n{}", screen.plain());
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!screen.plain().contains("to let it run"));
    std::thread::sleep(Duration::from_millis(500));
    writer.lock().unwrap().write_all(&win32_key(0x4B, 0x25, 0x0B, 0x1A)).unwrap();
    writer.lock().unwrap().write_all(b"exit\r").unwrap();
    wait_exit(&mut child, &screen);
}

/// Discreet: the question goes to the window's title bar, the answer is
/// typed unseen, and nothing of KeyJutsu's appears in the console the room
/// is watching.
#[test]
fn a_discreet_run_asks_in_the_title_bar_and_shows_nothing_in_the_console() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("run-discreet-critical");
    let _ = std::fs::remove_dir_all(&dir);
    let victim = dir.join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("keep.txt"), "x").unwrap();
    let stale = critical_snapshot(&dir, &victim, "2026-09-24T00:00:00Z");
    let snap = stale.display().to_string();
    let args = ["run", &snap, "--mode", "direct", "--clean", "--ephemeral", "--presentation", "discreet"];
    let (mut child, screen, writer) = launch_in(&args, None);
    assert!(
        screen.wait_for_raw("KeyJutsu is waiting: type REMOVE THE VICTIM and press Enter"),
        "{}",
        screen.plain()
    );
    writer.lock().unwrap().write_all(b"REMOVE THE VICTIM\r").unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while victim.exists() {
        assert!(Instant::now() < deadline, "the step never ran:\n{}", screen.plain());
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(screen.wait_for_raw("Ctrl+Alt+Shift+K gives you the keyboard back"), "{}", screen.plain());
    let seen = screen.plain();
    assert!(!seen.contains("CRITICAL ACTION"), "{seen}");
    assert!(!seen.contains("REMOVE THE VICTIM"), "the answer was shown: {seen}");
    std::thread::sleep(Duration::from_millis(500));
    writer.lock().unwrap().write_all(&win32_key(0x4B, 0x25, 0x0B, 0x1A)).unwrap();
    writer.lock().unwrap().write_all(b"exit\r").unwrap();
    assert!(wait_exit(&mut child, &screen).success(), "{}", screen.plain());
}
