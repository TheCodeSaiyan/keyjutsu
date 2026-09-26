//! The broker refuses altered commands, unknown plan hashes,
//! unauthorised operations and other protocol versions, and has no way to
//! run a command string. Over a real named pipe, unelevated: the checks do
//! not depend on elevation; elevation itself is tried in Windows Sandbox.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use keyjutsu_broker::pipe::ServerPipe;
use keyjutsu_broker::{
    Broker, BrokerClient, PROTOCOL, Request, Response, random_hex, read_frame, run_in_shell, serve,
    write_frame,
};
use keyjutsu_core::elevation::ElevatedRunner;
use keyjutsu_core::execution::StepOutcome;
use keyjutsu_core::plan::{ApprovalBook, ApprovedSnapshot, ValidPlan, parse_plan, seal};
use keyjutsu_core::validation::{Options, validate};
use serde_json::json;

const AT: &str = "2026-09-25T12:00:00Z";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("broker").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fwd(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// An Administrator step that leaves a file, and a standard one.
fn snapshot(dir: &Path, value: &str) -> ApprovedSnapshot {
    let file = fwd(&dir.join("admin.txt"));
    let draft = parse_plan(
        &json!({
            "schema_version": "1.0", "plan_id": "p", "task_id": "t",
            "target": {"id": "local", "kind": "local_windows"},
            "agent": {"name": "codex", "version": "1"},
            "steps": [
                {"id": "admin", "title": "Admin", "objective": "Needs Administrator.", "kind": "command",
                 "shell": {"kind": "pwsh"}, "privilege": "administrator",
                 "commands": [{"text": format!("Set-Content -LiteralPath {file} -Value {value}")}]},
                {"id": "plain", "title": "Plain", "objective": "Does not.", "kind": "command",
                 "shell": {"kind": "pwsh"}, "commands": [{"text": "Get-Date"}]}
            ]
        })
        .to_string(),
    )
    .unwrap();
    let report = validate(&draft, Options { dry_run: false, broker_available: true });
    assert!(report.not_ready(&draft).is_empty(), "{:#?}", report.steps);
    let v = ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    assert!(book.approve_all_except_critical(&v, AT).is_empty());
    seal(&v, &book, None, AT).unwrap()
}

/// A broker on a fresh pipe, as `keyjutsu-broker` runs one: it serves only
/// the process `expected_pid`.
fn start(
    snap: ApprovedSnapshot,
    secret: &str,
    expected_pid: u32,
) -> (String, std::thread::JoinHandle<Result<(), String>>) {
    let name = format!("keyjutsu-broker-test-{}", &random_hex()[..16]);
    let server = ServerPipe::create(&name).unwrap();
    let secret = secret.to_owned();
    let handle = std::thread::spawn(move || {
        keyjutsu_broker::accept_launcher(&server, expected_pid)?;
        let mut broker = Broker::new(
            snap,
            secret,
            Box::new(|s, step, _| run_in_shell(s, step).map(keyjutsu_broker::Done::Ran)),
        );
        let mut server = server;
        serve(&mut server, &mut broker).map_err(|e| e.to_string())
    });
    (name, handle)
}

fn refused(r: Result<impl std::fmt::Debug, String>, needle: &str) {
    match r {
        Err(reason) => assert!(reason.contains(needle), "refused, but for `{reason}`, not `{needle}`"),
        Ok(v) => panic!("expected a refusal containing `{needle}`, got {v:?}"),
    }
}

#[test]
fn an_approved_administrator_step_runs_and_nothing_else_does() {
    let dir = scratch("done-when");
    let snap = snapshot(&dir, "approved");
    let hashes = snap.step_hashes().clone();
    let (name, server) = start(snap.clone(), "s3cret", std::process::id());
    let client = BrokerClient::connect(&name, "s3cret", Duration::from_secs(5)).unwrap();

    // Altered command: the same step with a different command has a
    // different hash, and the broker's copy decides.
    let altered = snapshot(&scratch("altered"), "altered");
    refused(
        client.run_step(snap.snapshot_hash(), "admin", &altered.step_hashes()["admin"]),
        "not the approved version",
    );
    // Unknown plan.
    refused(
        client.run_step(altered.snapshot_hash(), "admin", &altered.step_hashes()["admin"]),
        "unknown plan",
    );
    // Unauthorised: a step that needs no elevation, a step that does not exist.
    refused(client.run_step(snap.snapshot_hash(), "plain", &hashes["plain"]), "does not need Administrator");
    refused(client.run_step(snap.snapshot_hash(), "nope", &hashes["admin"]), "no step `nope`");
    // There is no request that carries a command.
    let execute = json!({"kind": "execute", "command": "Remove-Item C:/Windows -Recurse"}).to_string();
    assert_eq!(
        client.ask_raw(execute.as_bytes()).unwrap(),
        Response::Refused { reason: "not a request this broker accepts".into() }
    );
    let smuggled = json!({"kind": "run_step", "snapshot_hash": snap.snapshot_hash(), "step": "admin",
        "step_hash": hashes["admin"], "command": "calc.exe"})
    .to_string();
    assert!(
        matches!(client.ask_raw(smuggled.as_bytes()).unwrap(), Response::Refused { .. }),
        "no extra fields"
    );
    assert!(!dir.join("admin.txt").exists(), "nothing ran so far");

    // The approved step, exactly as approved, runs.
    let run = client.run_step(snap.snapshot_hash(), "admin", &hashes["admin"]).unwrap();
    assert!(matches!(run.outcomes.as_slice(), [StepOutcome::Succeeded { .. }]), "{run:?}");
    assert_eq!(std::fs::read_to_string(dir.join("admin.txt")).unwrap().trim(), "approved");
    drop(client);
    server.join().unwrap().unwrap();
}

#[test]
fn another_protocol_version_is_refused_not_negotiated() {
    let dir = scratch("protocol");
    let (name, server) = start(snapshot(&dir, "x"), "s3cret", std::process::id());
    let mut pipe =
        std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\{name}")).unwrap();
    let hello =
        serde_json::to_vec(&Request::Hello { protocol: PROTOCOL + 1, secret: "s3cret".into() }).unwrap();
    write_frame(&mut pipe, &hello).unwrap();
    let answer: Response = serde_json::from_slice(&read_frame(&mut pipe).unwrap().unwrap()).unwrap();
    assert!(
        matches!(&answer, Response::Refused { reason }
            if reason.contains(&format!("protocol version {} ", PROTOCOL + 1))),
        "{answer:?}"
    );

    // Nor does it run anything before a successful hello.
    let run =
        json!({"kind": "run_step", "snapshot_hash": "x", "step": "admin", "step_hash": "x"}).to_string();
    write_frame(&mut pipe, run.as_bytes()).unwrap();
    let answer: Response = serde_json::from_slice(&read_frame(&mut pipe).unwrap().unwrap()).unwrap();
    assert_eq!(answer, Response::Refused { reason: "not authenticated".into() });
    pipe.flush().unwrap();
    drop(pipe);
    server.join().unwrap().unwrap();
}

#[test]
fn a_client_without_the_secret_or_from_another_process_is_refused() {
    let dir = scratch("identity");
    let (name, server) = start(snapshot(&dir, "x"), "s3cret", std::process::id());
    refused(BrokerClient::connect(&name, "guess", Duration::from_secs(5)).map(|_| ()), "not authenticated");
    server.join().unwrap().unwrap();

    // The broker expects a different process: this one is turned away.
    let (name, server) = start(snapshot(&dir, "x"), "s3cret", std::process::id() + 1);
    assert!(BrokerClient::connect(&name, "s3cret", Duration::from_secs(5)).is_err());
    assert!(server.join().unwrap().unwrap_err().contains("refused process"));
}

#[test]
fn a_pipe_name_cannot_be_taken_over() {
    let name = format!("keyjutsu-broker-test-{}", &random_hex()[..16]);
    let first = ServerPipe::create(&name).unwrap();
    assert!(ServerPipe::create(&name).is_err(), "a second pipe of the same name is refused");
    drop(first);
}

#[test]
fn the_broker_binary_refuses_a_snapshot_it_was_not_launched_for() {
    let dir = scratch("binary");
    let snap = snapshot(&dir, "x");
    let file = dir.join("snap.json");
    std::fs::write(&file, snap.to_json()).unwrap();
    let exe = env!("CARGO_BIN_EXE_keyjutsu-broker");
    let wrong = std::process::Command::new(exe)
        .args(["--pipe", "keyjutsu-broker-test-unused", "--client-pid", "1", "--snapshot"])
        .arg(&file)
        .args(["--snapshot-hash", &"0".repeat(64), "--secret", "s"])
        .status()
        .unwrap();
    assert_eq!(
        wrong.code(),
        Some(i32::from(keyjutsu_broker::exit::SNAPSHOT_NOT_LAUNCHED)),
        "a snapshot other than the launched one"
    );
    // An altered file does not verify at all.
    let text = snap.to_json().replace("Set-Content", "Remove-Item");
    std::fs::write(&file, text).unwrap();
    let altered = std::process::Command::new(exe)
        .args(["--pipe", "keyjutsu-broker-test-unused2", "--client-pid", "1", "--snapshot"])
        .arg(&file)
        .args(["--snapshot-hash", snap.snapshot_hash(), "--secret", "s"])
        .status()
        .unwrap();
    assert_eq!(
        altered.code(),
        Some(i32::from(keyjutsu_broker::exit::SNAPSHOT_NOT_VERIFIED)),
        "an altered snapshot file"
    );
}

/// A step with one artifact, pinned to `bytes`' hash.
fn artifact_step(bytes: &[u8]) -> keyjutsu_core::plan::model::Step {
    let sha = keyjutsu_core::plan::hash::sha256_hex(bytes);
    let draft = parse_plan(
        &json!({
            "schema_version": "1.0", "plan_id": "p", "task_id": "t",
            "target": {"id": "local", "kind": "local_windows"},
            "agent": {"name": "codex", "version": "1"},
            "steps": [{"id": "install", "title": "Install", "objective": "Uses a download.",
                       "kind": "command", "shell": {"kind": "pwsh"}, "privilege": "administrator",
                       "artifacts": [{"name": "tool.zip", "source": "https://example.com/tool.zip", "sha256": sha}],
                       "commands": [{"text": "Get-Item -LiteralPath $KJ_ARTIFACTS['tool.zip']"}]}]
        })
        .to_string(),
    )
    .unwrap();
    draft.plan().steps[0].clone()
}

fn stage(store: &Path, step: &keyjutsu_core::plan::model::Step, bytes: &[u8]) -> PathBuf {
    let a = &step.artifacts[0];
    let path = keyjutsu_core::artifacts::staged_path(store, a.sha256.as_ref().unwrap(), &a.name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    path
}

/// An elevated step is handed the checked copy in the broker's own folder,
/// never the operator's staged file, which anything unelevated can change.
#[test]
fn an_elevated_step_is_handed_only_a_checked_copy_the_operator_cannot_touch() {
    use keyjutsu_broker::hand_over;
    let dir = scratch("hand-over");
    let (store, into) = (dir.join("store"), dir.join("protected"));
    let step = artifact_step(b"the approved tool");
    let staged = stage(&store, &step, b"the approved tool");

    let line = hand_over(&step, &store, &into).unwrap().unwrap();
    let copy =
        keyjutsu_core::artifacts::staged_path(&into, step.artifacts[0].sha256.as_ref().unwrap(), "tool.zip");
    assert!(line.contains(&copy.display().to_string()), "{line}");
    assert!(!line.contains(&staged.display().to_string()), "the operator's copy was handed over: {line}");
    assert_eq!(std::fs::read(&copy).unwrap(), b"the approved tool");

    // Changed after staging: refused, whatever the operator's store says.
    let _ = std::fs::remove_dir_all(&into);
    std::fs::write(&staged, b"something else").unwrap();
    let refused = hand_over(&step, &store, &into).unwrap_err();
    assert!(refused.contains("changed since it was staged"), "{refused}");

    // Never staged, or never pinned: refused.
    let _ = std::fs::remove_dir_all(&store);
    assert!(hand_over(&step, &store, &into).unwrap_err().contains("not staged"));
    let mut unpinned = step.clone();
    unpinned.artifacts[0].sha256 = None;
    assert!(hand_over(&unpinned, &store, &into).unwrap_err().contains("not pinned"));
}

fn powershell(script: &str) -> String {
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// The broker restores its own capture, of what the step declared, and
/// nothing else; a capture changed to name something else is refused, and
/// a capture is used once. Run against HKCU, with an ordinary folder
/// standing in for the Administrators-only one.
#[test]
fn the_broker_restores_only_its_own_capture_of_what_the_step_declared() {
    use keyjutsu_broker::captures::{capture, restore};
    let dir = scratch("captures");
    let root = dir.join("root");
    let key = format!(r"HKCU:\Software\KeyJutsu-Tests\broker-{}", std::process::id());
    let value = format!(r"{key}\Value");
    // The test's key goes when the test ends, passed or not.
    struct Gone(String);
    impl Drop for Gone {
        fn drop(&mut self) {
            powershell(&format!(
                "Remove-Item -LiteralPath '{}' -Recurse -Force; $p = 'HKCU:\\Software\\KeyJutsu-Tests'; if (-not (Get-ChildItem -LiteralPath $p)) {{ Remove-Item -LiteralPath $p -ErrorAction SilentlyContinue }}",
                self.0
            ));
        }
    }
    let _gone = Gone(key.clone());
    let read = || powershell(&format!("(Get-ItemProperty -LiteralPath '{key}' -Name Value).Value"));
    powershell(&format!(
        "New-Item -Path '{key}' -Force | Out-Null; Set-ItemProperty -LiteralPath '{key}' -Name Value -Value before"
    ));

    let draft = parse_plan(
        &json!({
            "schema_version": "1.0", "plan_id": "p", "task_id": "t",
            "target": {"id": "local", "kind": "local_windows"},
            "agent": {"name": "codex", "version": "1"},
            "steps": [{"id": "change", "title": "Change", "objective": "Changes a value.", "kind": "command",
                       "shell": {"kind": "pwsh"}, "commands": [{"text": "Get-Date"}],
                       "reversibility": {"level": "full"},
                       "recovery": {"strategy": "restore_captured_state",
                                    "capture": [{"kind": "registry_value", "target": value}]}}]
        })
        .to_string(),
    )
    .unwrap();
    let report = validate(&draft, Options { dry_run: false, broker_available: true });
    let v = ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    book.approve_all_except_critical(&v, AT);
    let snap = seal(&v, &book, None, AT).unwrap();
    let step = snap.plan().step("change").unwrap().clone();

    capture(&root, &snap, &step, AT.into()).unwrap();
    powershell(&format!("Set-ItemProperty -LiteralPath '{key}' -Name Value -Value after"));
    assert_eq!(read(), "after");

    // Its capture changed to name something the step never declared.
    let kept = root.join("captures").join(snap.snapshot_hash()).join("change").join("capture.json");
    let honest = std::fs::read_to_string(&kept).unwrap();
    // In the JSON each backslash is written twice.
    let forged = honest.replace(&value.replace('\\', r"\\"), r"HKCU:\\Software\\Elsewhere\\Run");
    assert_ne!(forged, honest);
    std::fs::write(&kept, forged).unwrap();
    let refused = restore(&root, &snap, &step).unwrap_err();
    assert!(refused.contains("was not declared"), "{refused}");
    assert_eq!(read(), "after", "nothing was written");

    std::fs::write(&kept, honest).unwrap();
    let checks = restore(&root, &snap, &step).unwrap();
    assert!(checks.iter().all(|c| c.passed == Some(true)), "{checks:?}");
    assert_eq!(read(), "before");
    assert!(!kept.exists(), "a capture is used once");
    assert!(restore(&root, &snap, &step).unwrap_err().contains("captured nothing"));
}
