//! Approved snapshots executed through real pwsh sessions, in
//! each mode, from the same snapshot. Plans are validated, approved and
//! sealed exactly as the CLI does it; nothing here is mocked.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keyjutsu_core::execute::{
    Checkpoint, Driver, ExecuteOptions, ExecutionEvent, ForwardingSink, InProgress, Outcome, execute,
};
use keyjutsu_core::execution::{Cadence, ExecutionMode, ExecutionState, PerformanceConfig};
use keyjutsu_core::headless::{Collector, strip_ansi};
use keyjutsu_core::plan::{ApprovalBook, ApprovedSnapshot, parse_plan, seal};
use keyjutsu_core::terminal::{KeyChord, KeyName, ProfileMode, ShellKind};
use keyjutsu_core::validation::{Options, validate};
use keyjutsu_core::{Session, SessionOptions};
use serde_json::{Value, json};

const AT: &str = "2026-09-25T04:00:00Z";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("execute").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fwd(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

fn step(id: &str, command: &str) -> Value {
    json!({"id": id, "title": id, "objective": "Test.", "kind": "command", "shell": {"kind": "pwsh"},
           "commands": [{"text": command}]})
}

fn plan(steps: Value, edges: Value) -> Value {
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": steps, "edges": edges
    })
}

/// Validate, approve every step and seal, as `keyjutsu plan approve` does.
fn approve(v: &Value) -> ApprovedSnapshot {
    let draft = parse_plan(&v.to_string()).unwrap();
    let report = validate(&draft, Options { dry_run: false, ..Options::default() });
    let not_ready = report.not_ready(&draft);
    assert!(not_ready.is_empty(), "not ready: {not_ready:?} {:#?}", report.steps);
    let validated =
        keyjutsu_core::plan::ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    assert!(book.approve_all_except_critical(&validated, AT).is_empty());
    seal(&validated, &book, None, AT).unwrap()
}

struct Terminal {
    session: Session,
    events: std::sync::mpsc::Receiver<keyjutsu_core::SessionEvent>,
    out: Arc<Collector>,
}

fn terminal() -> Terminal {
    let out = Arc::new(Collector::new());
    let (tx, events) = channel();
    let sink = Arc::new(ForwardingSink { inner: out.clone(), events: Mutex::new(tx) });
    let mut o = SessionOptions::new(ShellKind::Pwsh);
    o.profile = ProfileMode::Clean;
    o.intercept_cursor_queries = true;
    let session = Session::open(o, sink).unwrap();
    assert!(session.wait_ready(Duration::from_secs(30)));
    Terminal { session, events, out }
}

fn run(
    t: &Terminal,
    snap: &ApprovedSnapshot,
    options: &ExecuteOptions,
    resume: Option<Checkpoint>,
) -> (Outcome, Checkpoint, Vec<ExecutionEvent>) {
    let seen = Mutex::new(Vec::new());
    let (outcome, checkpoint) = execute(
        &Driver { session: &t.session, events: &t.events },
        snap,
        resume,
        options,
        &|| AT.to_owned(),
        &|e| seen.lock().unwrap().push(e),
    );
    (outcome, checkpoint, seen.into_inner().unwrap())
}

fn mode(m: ExecutionMode) -> ExecuteOptions {
    ExecuteOptions {
        mode: Some(m),
        base: PerformanceConfig {
            cadence: Cadence { base_ms: 2, variance_ms: 1, punctuation_pause_ms: 2, boundary_pause_ms: 20 },
            ..PerformanceConfig::default()
        },
        ..ExecuteOptions::default()
    }
}

/// Three steps that leave evidence on disk, with internal checks.
fn three_steps(dir: &Path) -> Value {
    let file = fwd(&dir.join("note.txt"));
    let mut make = step("make", &format!("New-Item -ItemType File -Force -Path {file} | Out-Null"));
    make["internal_validation"] = json!([{"path_exists": {"path": file}}]);
    let mut write = step("write", &format!("Add-Content -LiteralPath {file} -Value written"));
    write["internal_validation"] = json!([{"exit_code": {"equals": 0}}]);
    let mut show = step("show", &format!("Get-Content -LiteralPath {file}"));
    show["visible_validation"] = json!([{"text": "Get-Date -Format o"}]);
    plan(json!([make, write, show]), json!([]))
}

#[test]
fn the_same_snapshot_runs_in_every_mode() {
    for m in [
        ExecutionMode::Direct,
        ExecutionMode::AutoPerformance,
        ExecutionMode::Performance,
        ExecutionMode::Assisted,
    ] {
        let dir = scratch(&format!("modes-{m:?}"));
        let snap = approve(&three_steps(&dir));
        let t = terminal();
        // In Performance and Assisted modes someone has to press keys.
        let done = Arc::new(AtomicBool::new(false));
        let masher = matches!(m, ExecutionMode::Performance | ExecutionMode::Assisted).then(|| {
            let (session, done) = (t.session.clone(), done.clone());
            std::thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    // As the CLI and desktop do during a run: a key with no
                    // performance owning the keyboard is held back, not typed
                    // into the shell (it would dirty the line before arming).
                    if session.snapshot().is_some_and(|s| s.owns_input) {
                        let _ = session.key(&KeyChord::char('z'));
                    }
                    std::thread::sleep(Duration::from_millis(3));
                }
            })
        });
        let (outcome, checkpoint, events) = run(&t, &snap, &mode(m), None);
        done.store(true, Ordering::SeqCst);
        if let Some(h) = masher {
            h.join().unwrap();
        }
        assert_eq!(outcome, Outcome::Complete, "{m:?}: {:?}\n{}", events, t.out.plain_output());
        assert_eq!(checkpoint.runs.len(), 3);
        assert!(
            checkpoint.runs.iter().all(|r| r.succeeded && r.checks.iter().all(|c| c.passed != Some(false)))
        );
        assert_eq!(std::fs::read_to_string(dir.join("note.txt")).unwrap().trim(), "written", "{m:?}");
        assert!(checkpoint.in_progress.is_none());
        let plain = t.out.plain_output();
        assert!(!plain.contains("zz"), "{m:?}: mashed keys reached the shell");
        t.session.close();
    }
}

#[test]
fn a_failure_halts_the_plan_and_reports_expected_versus_actual() {
    let dir = scratch("failure");
    let marker = fwd(&dir.join("never.txt"));
    let v = plan(
        json!([
            step("first", "Get-Date"),
            step("breaks", "cmd /c exit 3"),
            step("after", &format!("New-Item -ItemType File -Path {marker}"))
        ]),
        json!([]),
    );
    let snap = approve(&v);
    let t = terminal();
    let (outcome, checkpoint, _) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    match outcome {
        Outcome::Failed { step, actual, .. } => {
            assert_eq!(step, "breaks");
            assert!(actual.contains('3'), "{actual}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!dir.join("never.txt").exists(), "the step after the failure ran");
    assert_eq!(
        checkpoint.runs.iter().map(|r| (r.step.as_str(), r.succeeded)).collect::<Vec<_>>(),
        [("first", true), ("breaks", false)]
    );
    t.session.close();
}

#[test]
fn a_failing_internal_check_fails_the_step() {
    let dir = scratch("check");
    let missing = fwd(&dir.join("not-there.txt"));
    let mut s = step("look", "Get-Date");
    s["internal_validation"] = json!([{"path_exists": {"path": missing}}]);
    let snap = approve(&plan(json!([s]), json!([])));
    let t = terminal();
    let (outcome, _, _) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    match outcome {
        Outcome::Failed { expected, actual, .. } => {
            assert!(expected.contains("not-there.txt"), "{expected}");
            assert_eq!(actual, "does not exist");
        }
        other => panic!("{other:?}"),
    }
    // The keyboard is the operator's again, as after any failure.
    assert!(t.session.write_input(b"x").is_ok(), "the failed run still holds the keyboard");
    t.session.close();
}

#[test]
fn a_repaired_plan_resumes_without_rerunning_unchanged_steps() {
    let dir = scratch("resume");
    let log = fwd(&dir.join("log.txt"));
    let first = step("first", &format!("Add-Content -LiteralPath {log} -Value first"));
    let broken = plan(json!([first.clone(), step("second", "cmd /c exit 4")]), json!([]));
    let snap = approve(&broken);
    let t = terminal();
    let (outcome, checkpoint, _) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    assert!(matches!(outcome, Outcome::Failed { .. }));

    // Revised, revalidated and re-approved: a new snapshot.
    let fixed = plan(json!([first, step("second", "cmd /c exit 0")]), json!([]));
    let snap2 = approve(&fixed);
    let (outcome, checkpoint2, events) = run(&t, &snap2, &mode(ExecutionMode::Direct), Some(checkpoint));
    assert_eq!(outcome, Outcome::Complete, "{events:?}");
    assert!(events.contains(&ExecutionEvent::StepCarried { step: "first".into() }));
    let lines = std::fs::read_to_string(dir.join("log.txt")).unwrap();
    assert_eq!(lines.lines().count(), 1, "the unchanged step ran again");
    assert_eq!(checkpoint2.snapshot_hash, snap2.snapshot_hash());
    t.session.close();
}

#[test]
fn branches_follow_real_outcomes() {
    let dir = scratch("branch");
    let taken = fwd(&dir.join("taken.txt"));
    let not_taken = fwd(&dir.join("not-taken.txt"));
    let v = plan(
        json!([
            step("probe", "Get-Date"),
            step("yes", &format!("New-Item -ItemType File -Path {taken}")),
            step("no", &format!("New-Item -ItemType File -Path {not_taken}"))
        ]),
        json!([
            {"from": "probe", "to": "yes", "when": {"step_outcome": {"step": "probe", "is": "succeeded"}}},
            {"from": "probe", "to": "no", "when": {"not": {"step_outcome": {"step": "probe", "is": "succeeded"}}}}
        ]),
    );
    let snap = approve(&v);
    let t = terminal();
    let (outcome, checkpoint, _) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    assert_eq!(outcome, Outcome::Complete);
    assert!(dir.join("taken.txt").exists());
    assert!(!dir.join("not-taken.txt").exists());
    assert_eq!(checkpoint.runs.len(), 2);
    t.session.close();
}

#[test]
fn a_step_left_in_doubt_blocks_until_the_operator_settles_it() {
    let dir = scratch("doubt");
    let log = fwd(&dir.join("log.txt"));
    let snap = approve(&plan(
        json!([step("a", &format!("Add-Content -LiteralPath {log} -Value a")), step("b", "Get-Date")]),
        json!([]),
    ));
    let hash = snap.step_hashes()["a"].clone();
    let mut crashed = Checkpoint::new(snap.snapshot_hash());
    crashed.in_progress = Some(InProgress { step: "a".into(), step_hash: hash, started_at: AT.into() });

    let t = terminal();
    let (outcome, _, _) = run(&t, &snap, &mode(ExecutionMode::Direct), Some(crashed.clone()));
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("effect is unknown")),
        "{outcome:?}"
    );

    let settled =
        ExecuteOptions { settled: BTreeMap::from([("a".to_owned(), true)]), ..mode(ExecutionMode::Direct) };
    let (outcome, checkpoint, _) = run(&t, &snap, &settled, Some(crashed));
    assert_eq!(outcome, Outcome::Complete);
    assert!(!dir.join("log.txt").exists(), "a step settled as done is not run again");
    assert_eq!(checkpoint.runs.len(), 2);
    t.session.close();
}

#[test]
fn an_unvalidated_snapshot_is_refused() {
    let draft = parse_plan(&plan(json!([step("a", "Get-Date")]), json!([])).to_string()).unwrap();
    let mut book = ApprovalBook::new();
    book.approve_all_except_critical(&draft, AT);
    let snap = seal(&draft, &book, None, AT).unwrap();
    let t = terminal();
    let (outcome, _, _) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("not validated")),
        "{outcome:?}"
    );
    t.session.close();
}

#[test]
fn checkpoints_are_written_as_the_plan_runs() {
    let dir = scratch("checkpoint");
    let path = dir.join("run.checkpoint.json");
    let snap = approve(&plan(json!([step("a", "Get-Date"), step("b", "Get-Date")]), json!([])));
    let t = terminal();
    let options = ExecuteOptions { checkpoint: Some(path.clone()), ..mode(ExecutionMode::Direct) };
    let (outcome, _, _) = run(&t, &snap, &options, None);
    assert_eq!(outcome, Outcome::Complete);
    let saved = Checkpoint::load(&path).unwrap();
    assert_eq!(saved.runs.len(), 2);
    assert_eq!(saved.snapshot_hash, snap.snapshot_hash());
    assert!(saved.in_progress.is_none());
    t.session.close();
}

/// A failed step carries what it printed, and nothing from the steps before
/// it, so the agent asked to fix it reads the real error.
#[test]
fn a_failed_step_carries_what_it_printed() {
    let dir = scratch("failure-output");
    let missing = fwd(&dir.join("never-made.txt"));
    let mut first = step("first", "Write-Output 'from-the-first-step'");
    first["internal_validation"] = json!([{"exit_code": {"equals": 0}}]);
    let mut second = step("second", "Write-Output 'the-widget-is-missing'");
    second["internal_validation"] = json!([{"path_exists": {"path": missing}}]);
    let snap = approve(&plan(json!([first, second]), json!([])));
    let t = terminal();
    let (outcome, _, _) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    match outcome {
        Outcome::Failed { step, output, .. } => {
            assert_eq!(step, "second");
            assert!(output.contains("the-widget-is-missing"), "{output:?}");
            assert!(!output.contains("from-the-first-step"), "only this step's output: {output:?}");
            assert!(!output.contains('\x1b'), "no colour codes: {output:?}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    t.session.close();
}

/// Failure injection: the checkpoint cannot be written. The step would leave
/// no record of having started, so a crash during it would look like it
/// never ran; it is not started.
#[test]
fn a_step_that_cannot_be_recorded_as_starting_does_not_run() {
    let dir = scratch("unrecordable");
    let marker = dir.join("ran.txt");
    let snap = approve(&plan(
        json!([step("make", &format!("New-Item -ItemType File -Path {} | Out-Null", fwd(&marker)))]),
        json!([]),
    ));
    let t = terminal();
    let options = ExecuteOptions {
        checkpoint: Some(dir.join("no-such-folder").join("run.checkpoint.json")),
        ..mode(ExecutionMode::Direct)
    };
    let (outcome, _, events) = run(&t, &snap, &options, None);
    match outcome {
        Outcome::Blocked { reason } => assert!(reason.contains("has not run"), "{reason}"),
        other => panic!("expected Blocked, got {other:?}"),
    }
    assert!(!events.iter().any(|e| matches!(e, ExecutionEvent::StepStarting { .. })));
    std::thread::sleep(Duration::from_millis(500));
    assert!(!marker.exists(), "the step ran");
    t.session.close();
}

/// Failure injection: the elevation broker's pipe breaks during an
/// Administrator step. The broker may have started it, so the step is left
/// in doubt, never counted as done, and the plan stops.
#[test]
fn a_broker_that_dies_mid_step_leaves_the_step_in_doubt() {
    struct Broken;
    impl keyjutsu_core::elevation::ElevatedRunner for Broken {
        fn run_step(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<keyjutsu_core::elevation::ElevatedRun, String> {
            Err("the pipe was closed before the step finished (os error 109)".into())
        }
    }
    if keyjutsu_core::elevation::is_elevated() {
        eprintln!("skipped: run elevated, Administrator steps need no broker");
        return;
    }
    let dir = scratch("broker-dies");
    let path = dir.join("run.checkpoint.json");
    let mut admin = step("admin", "Get-Service -Name Winmgmt");
    admin["privilege"] = json!("administrator");
    let draft = parse_plan(&plan(json!([admin, step("after", "Get-Date")]), json!([])).to_string()).unwrap();
    let report = validate(&draft, Options { dry_run: false, broker_available: true });
    let validated =
        keyjutsu_core::plan::ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    assert!(book.approve_all_except_critical(&validated, AT).is_empty());
    let snap = seal(&validated, &book, None, AT).unwrap();

    let t = terminal();
    let options = ExecuteOptions {
        checkpoint: Some(path.clone()),
        elevated_runner: Some(Arc::new(Broken)),
        ..mode(ExecutionMode::Direct)
    };
    let (outcome, checkpoint, _) = run(&t, &snap, &options, None);
    match outcome {
        Outcome::Blocked { reason } => assert!(reason.contains("pipe was closed"), "{reason}"),
        other => panic!("expected Blocked, got {other:?}"),
    }
    assert!(checkpoint.runs.is_empty(), "nothing is counted as done");
    assert_eq!(checkpoint.in_progress.as_ref().map(|p| p.step.as_str()), Some("admin"));
    assert_eq!(Checkpoint::load(&path).unwrap().in_progress.map(|p| p.step), Some("admin".into()));
    t.session.close();
}

#[test]
fn a_changed_step_runs_again_even_though_it_succeeded_before() {
    let dir = scratch("changed");
    let log = fwd(&dir.join("log.txt"));
    let v1 = plan(json!([step("a", &format!("Add-Content -LiteralPath {log} -Value one"))]), json!([]));
    let t = terminal();
    let (outcome, checkpoint, _) = run(&t, &approve(&v1), &mode(ExecutionMode::Direct), None);
    assert_eq!(outcome, Outcome::Complete);

    // Same step id, different command: its hash differs, so the old success does not count.
    let v2 = plan(json!([step("a", &format!("Add-Content -LiteralPath {log} -Value two"))]), json!([]));
    let (outcome, _, events) = run(&t, &approve(&v2), &mode(ExecutionMode::Direct), Some(checkpoint));
    assert_eq!(outcome, Outcome::Complete);
    assert!(!events.iter().any(|e| matches!(e, ExecutionEvent::StepCarried { .. })), "{events:?}");
    assert_eq!(
        std::fs::read_to_string(dir.join("log.txt")).unwrap().lines().collect::<Vec<_>>(),
        ["one", "two"]
    );
    t.session.close();
}

#[test]
fn a_step_can_override_the_run_mode() {
    let dir = scratch("override");
    let file = fwd(&dir.join("direct.txt"));
    let mut direct = step("direct", &format!("Set-Content -LiteralPath {file} -Value done"));
    direct["execution_mode"] = json!("direct");
    let snap = approve(&plan(json!([direct]), json!([])));
    let t = terminal();
    // Run in Performance mode with nobody pressing keys: only the step's own
    // Direct mode can finish it.
    let (outcome, _, events) = run(&t, &snap, &mode(ExecutionMode::Performance), None);
    assert_eq!(outcome, Outcome::Complete, "{events:?}");
    assert_eq!(std::fs::read_to_string(dir.join("direct.txt")).unwrap().trim(), "done");
    t.session.close();
}

#[test]
fn a_shell_that_exits_mid_step_is_never_a_success() {
    let dir = scratch("exits");
    let log = fwd(&dir.join("after.txt"));
    let snap = approve(&plan(
        json!([
            step("leave", "[Environment]::Exit(0)"),
            step("after", &format!("Set-Content -LiteralPath {log} -Value ran"))
        ]),
        json!([]),
    ));
    let t = terminal();
    let (outcome, checkpoint, events) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    assert_eq!(outcome, Outcome::Aborted { step: Some("leave".into()), in_doubt: true }, "{events:?}");
    assert!(checkpoint.runs.is_empty(), "{checkpoint:?}");
    assert_eq!(checkpoint.in_progress.map(|p| p.step).as_deref(), Some("leave"));
    assert!(!dir.join("after.txt").exists());
}

fn check(v: Value) -> keyjutsu_core::plan::model::Check {
    serde_json::from_value(v).unwrap()
}

#[test]
fn runtime_checks_look_at_the_machine_itself() {
    use keyjutsu_core::execute::run_check;
    let dir = scratch("checks");
    let file = dir.join("settings.json");
    std::fs::write(&file, r#"{"backend": {"kind": "wsl"}}"#).unwrap();
    let path = fwd(&file);
    let digest = keyjutsu_core::plan::hash::sha256_hex(&std::fs::read(&file).unwrap());

    let passed = |c: Value| run_check(&check(c), Some(0)).passed;
    assert_eq!(passed(json!({"file_sha256": {"path": path, "sha256": digest}})), Some(true));
    assert_eq!(passed(json!({"file_sha256": {"path": path, "sha256": "00"}})), Some(false));
    assert_eq!(
        passed(json!({"json_value": {"path": path, "pointer": "/backend/kind", "equals": "wsl"}})),
        Some(true)
    );
    assert_eq!(
        passed(json!({"json_value": {"path": path, "pointer": "/backend/kind", "equals": "hyperv"}})),
        Some(false)
    );
    assert_eq!(
        passed(json!({"json_value": {"path": path, "pointer": "/missing", "equals": 1}})),
        Some(false)
    );
    assert_eq!(passed(json!({"exit_code": {"equals": 0}})), Some(true));
    assert_eq!(
        run_check(&check(json!({"exit_code": {"equals": 0}})), None).passed,
        None,
        "cmd: unknown, not passed"
    );

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    assert_eq!(passed(json!({"tcp_port_open": {"host": "127.0.0.1", "port": port}})), Some(true));
    drop(listener);
    assert_eq!(passed(json!({"tcp_port_open": {"host": "127.0.0.1", "port": port}})), Some(false));
}

#[test]
fn a_check_is_waited_for_up_to_its_timeout() {
    let dir = scratch("wait");
    let late = dir.join("late.txt");
    let mut s = step("start", "Get-Date | Out-Null");
    s["internal_validation"] = json!([{"path_exists": {"path": fwd(&late)}}, {"timeout_seconds": 20}]);
    let snap = approve(&plan(json!([s]), json!([])));
    let t = terminal();
    let writer = {
        let late = late.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            std::fs::write(late, "here").unwrap();
        })
    };
    let (outcome, _, events) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    writer.join().unwrap();
    assert_eq!(outcome, Outcome::Complete, "{events:?}");
    assert!(events.iter().any(|e| matches!(e, ExecutionEvent::Waiting { .. })), "it had to wait: {events:?}");
    t.session.close();
}

const SECRET: &str = "kj-S3cret-7x";

/// A plan that asks for a token, then uses it without printing it.
fn credential_plan() -> Value {
    let ask = json!({"id": "ask", "title": "Registry token", "objective": "Get the token.", "kind": "credential",
        "shell": {"kind": "pwsh"}, "execution_mode": "user_input",
        "credential": {"variable": "KJ_TOKEN", "prompt": "Token for the test", "kind": "secret"}});
    let mut used =
        step("use", "'length ' + [System.Net.NetworkCredential]::new('', $KJ_TOKEN).Password.Length");
    used["internal_validation"] = json!([{"exit_code": {"equals": 0}}]);
    plan(json!([ask, used]), json!([]))
}

/// The operator: waits for the credential step, presses Enter to start it,
/// then types the secret into the shell's prompt once it is showing.
/// Then it goes back to mashing for the steps after, until `done`.
fn operator(t: &Terminal, secret: &'static str, done: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    let (session, out) = (t.session.clone(), t.out.clone());
    std::thread::spawn(move || {
        // Giving up closes the session, so the run ends instead of waiting.
        let give_up = |why: &str| -> ! {
            session.close();
            panic!("{why}:\n{}", out.plain_output());
        };
        let asking =
            || session.snapshot().is_some_and(|s| s.asks_operator && s.state == ExecutionState::Armed);
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !asking() {
            if std::time::Instant::now() > deadline {
                give_up("the credential step never came");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Keys still being mashed start nothing; only Enter does.
        for c in "zzzz".chars() {
            session.key(&KeyChord::char(c)).unwrap();
        }
        if strip_ansi(&out.snapshot().output).contains("Token for the test") {
            give_up("mashed keys started the credential prompt");
        }
        session.key(&KeyChord::plain(KeyName::Enter)).unwrap();
        if !out
            .wait_until(Duration::from_secs(30), |c| strip_ansi(&c.output).contains("Token for the test: "))
        {
            give_up("the prompt never showed");
        }
        for c in secret.chars() {
            session.key(&KeyChord::char(c)).unwrap();
        }
        session.key(&KeyChord::plain(KeyName::Enter)).unwrap();
        while !done.load(Ordering::SeqCst) {
            if session.snapshot().is_some_and(|s| s.owns_input) {
                let _ = session.key(&KeyChord::char('q'));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })
}

#[test]
fn a_credential_is_entered_and_used_without_being_seen_or_kept() {
    let dir = scratch("credential");
    let path = dir.join("run.checkpoint.json");
    let plan_json = credential_plan();
    let snap = approve(&plan_json);
    let t = terminal();
    let done = Arc::new(AtomicBool::new(false));
    let answer = operator(&t, SECRET, done.clone());
    let options = ExecuteOptions { checkpoint: Some(path.clone()), ..mode(ExecutionMode::Performance) };
    let (outcome, checkpoint, events) = run(&t, &snap, &options, None);
    done.store(true, Ordering::SeqCst);
    answer.join().unwrap();
    assert_eq!(outcome, Outcome::Complete, "{events:?}\n{}", t.out.plain_output());
    assert!(events.iter().any(|e| matches!(e, ExecutionEvent::CredentialRequired { .. })));

    // It was used: the next step saw a value of the right length.
    let screen = t.out.plain_output();
    assert!(screen.contains(&format!("length {}", SECRET.len())), "{screen}");

    // And it was forgotten when the plan ended.
    t.session.disarm();
    t.session.write_input(b"'still set: ' + (Test-Path variable:KJ_TOKEN)\r").unwrap();
    assert!(
        t.out.wait_until(Duration::from_secs(20), |c| strip_ansi(&c.output).contains("still set: False"))
    );
    t.session.write_input(b"'history: ' + ((Get-History | Out-String) -replace '\\s+', ' ')\r").unwrap();
    assert!(t.out.wait_until(Duration::from_secs(20), |c| strip_ansi(&c.output).contains("history: ")));
    std::thread::sleep(Duration::from_millis(500));

    // Nowhere does the secret appear: not on screen (or anywhere in the raw
    // terminal output), history, the checkpoint, the events or the plan.
    let raw = t.out.snapshot().output;
    assert!(!raw.contains(SECRET), "the secret reached the terminal output");
    assert!(!std::fs::read_to_string(&path).unwrap().contains(SECRET));
    assert!(!serde_json::to_string(&checkpoint).unwrap().contains(SECRET));
    assert!(!format!("{events:?}").contains(SECRET));
    assert!(!snap.to_json().contains(SECRET) && !plan_json.to_string().contains(SECRET));
    t.session.close();
}

#[test]
fn a_resumed_run_asks_for_the_credential_again() {
    let snap = approve(&credential_plan());
    let t = terminal();
    let answer = operator(&t, SECRET, Arc::new(AtomicBool::new(true)));
    let (outcome, checkpoint, _) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    answer.join().unwrap();
    assert_eq!(outcome, Outcome::Complete);
    t.session.close();

    // A new shell: the old one's variable is gone, so the step must run again.
    let t = terminal();
    let answer = operator(&t, SECRET, Arc::new(AtomicBool::new(true)));
    let (outcome, _, events) = run(&t, &snap, &mode(ExecutionMode::Direct), Some(checkpoint));
    answer.join().unwrap();
    assert_eq!(outcome, Outcome::Complete, "{events:?}");
    assert!(
        events.iter().any(|e| matches!(e, ExecutionEvent::CredentialRequired { .. })),
        "the credential was carried over: {events:?}"
    );
    t.session.close();
}

#[test]
fn a_credential_step_is_refused_in_cmd() {
    let mut v = credential_plan();
    v["steps"][0]["shell"]["kind"] = json!("cmd");
    v["steps"][1] = json!({"id": "use", "title": "use", "objective": "Test.", "kind": "command",
        "shell": {"kind": "cmd"}, "commands": [{"text": "echo hi"}]});
    let draft = parse_plan(&v.to_string()).unwrap();
    let report = validate(&draft, Options { dry_run: false, ..Options::default() });
    let validated =
        keyjutsu_core::plan::ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    book.approve_all_except_critical(&validated, AT);
    let refused = match seal(&validated, &book, None, AT) {
        Err(_) => true,
        Ok(snap) => keyjutsu_core::execute::preflight(&snap).is_err_and(|e| e.contains("needs PowerShell")),
    };
    assert!(refused, "a cmd credential step must not run");
}

#[test]
fn the_credential_command_is_exactly_the_shells_own_masked_prompt() {
    use keyjutsu_core::execute::{credential_command, forget_command};
    use keyjutsu_core::plan::model::{CredentialKind, CredentialRequest};
    let mut r = CredentialRequest {
        variable: "TOKEN".into(),
        prompt: "Bob's \u{2019}token\u{2019}".into(),
        kind: CredentialKind::Secret,
        username: None,
        target_id: None,
    };
    // Every kind of single quote is doubled, so the prompt cannot end the string.
    assert_eq!(
        credential_command(&r),
        "$TOKEN = Read-Host -AsSecureString -Prompt 'Bob''s \u{2019}\u{2019}token\u{2019}\u{2019}'"
    );
    r.kind = CredentialKind::UsernameAndPassword;
    r.prompt = "Registry".into();
    r.username = Some("ci'bot".into());
    assert_eq!(credential_command(&r), "$TOKEN = Get-Credential -Message 'Registry' -UserName 'ci''bot'");
    assert_eq!(
        forget_command(&["A".into(), "B".into()]),
        "Remove-Variable -Name A,B -Scope Global -ErrorAction Ignore"
    );
}

#[test]
fn a_user_name_and_password_are_asked_for_in_turn() {
    let ask = json!({"id": "ask", "title": "Sign in", "objective": "Get a credential.", "kind": "credential",
        "shell": {"kind": "pwsh"}, "execution_mode": "user_input",
        "credential": {"variable": "KJ_CRED", "prompt": "Account for the test", "kind": "username_and_password"}});
    let used = step(
        "use",
        "'user ' + $KJ_CRED.UserName + ' length ' + $KJ_CRED.GetNetworkCredential().Password.Length",
    );
    let snap = approve(&plan(json!([ask, used]), json!([])));
    let t = terminal();
    let (session, out) = (t.session.clone(), t.out.clone());
    let answer = std::thread::spawn(move || {
        let seen = |text: &'static str| {
            if !out.wait_until(Duration::from_secs(30), |c| strip_ansi(&c.output).contains(text)) {
                session.close();
                panic!("never saw {text:?}:\n{}", out.plain_output());
            }
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !session.snapshot().is_some_and(|s| s.asks_operator) {
            if std::time::Instant::now() > deadline {
                session.close();
                panic!("the credential step never came:\n{}", out.plain_output());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        session.key(&KeyChord::plain(KeyName::Enter)).unwrap();
        seen("User:");
        for c in "bob".chars() {
            session.key(&KeyChord::char(c)).unwrap();
        }
        session.key(&KeyChord::plain(KeyName::Enter)).unwrap();
        seen("Password for user bob:");
        for c in "pa55word".chars() {
            session.key(&KeyChord::char(c)).unwrap();
        }
        session.key(&KeyChord::plain(KeyName::Enter)).unwrap();
    });
    let (outcome, _, events) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    answer.join().unwrap();
    assert_eq!(outcome, Outcome::Complete, "{events:?}\n{}", t.out.plain_output());
    let screen = t.out.plain_output();
    assert!(screen.contains("user bob length 8"), "{screen}");
    assert!(!t.out.snapshot().output.contains("pa55word"));
    t.session.close();
}

#[test]
fn a_critical_step_is_confirmed_again_just_before_it_runs() {
    use keyjutsu_core::execute::{CriticalConfirmation, CriticalGate};
    let dir = scratch("critical-gate");
    let victim = dir.join("victim");
    let mut s = step("wipe", &format!("Remove-Item -Recurse -Force -LiteralPath {}", fwd(&victim)));
    s["title"] = json!("Remove the victim folder");
    let v = plan(json!([s]), json!([]));
    let draft = parse_plan(&v.to_string()).unwrap();
    let report = validate(&draft, Options { dry_run: false, ..Options::default() });
    assert!(report.not_ready(&draft).is_empty(), "{:#?}", report.steps);
    let validated =
        keyjutsu_core::plan::ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    assert_eq!(book.approve_all_except_critical(&validated, AT), ["wipe"], "KeyJutsu rates it critical");
    book.approve(&validated, "wipe", AT, Some("REMOVE THE VICTIM FOLDER")).unwrap();
    let snap = seal(&validated, &book, None, AT).unwrap();

    let asked: Arc<Mutex<Vec<CriticalConfirmation>>> = Arc::default();
    let with = |answer: Option<&'static str>| {
        let asked = asked.clone();
        let gate: CriticalGate = Arc::new(move |c: &CriticalConfirmation| {
            asked.lock().unwrap().push(c.clone());
            answer.map(str::to_owned)
        });
        ExecuteOptions { critical_gate: Some(gate), ..mode(ExecutionMode::Direct) }
    };
    let t = terminal();
    for answer in [None, Some("yes"), Some("REMOVE THE VICTIM")] {
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("keep.txt"), "x").unwrap();
        let (outcome, _, _) = run(&t, &snap, &with(answer), None);
        assert!(
            matches!(&outcome, Outcome::Blocked { reason } if reason.contains("not confirmed")),
            "{answer:?}: {outcome:?}"
        );
        assert!(victim.join("keep.txt").exists(), "{answer:?}: it ran without confirmation");
    }
    let (outcome, _, _) = run(&t, &snap, &with(Some("REMOVE THE VICTIM FOLDER")), None);
    assert_eq!(outcome, Outcome::Complete);
    assert!(!victim.exists());
    t.session.close();

    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 4);
    assert_eq!(asked[0].phrase, "REMOVE THE VICTIM FOLDER");
    assert!(asked[0].commands[0].contains("Remove-Item -Recurse"));
    assert!(asked[0].recovery.contains("cannot undo"), "{:?}", asked[0].recovery);
}

#[test]
fn a_step_runs_in_its_working_directory() {
    let dir = scratch("working-directory");
    let inner = dir.join("inner folder's");
    std::fs::create_dir_all(&inner).unwrap();
    let mut s = step("here", "Set-Content -LiteralPath made-here.txt -Value x");
    s["working_directory"] = json!(fwd(&inner));
    let snap = approve(&plan(json!([s]), json!([])));
    let t = terminal();
    let (outcome, _, _) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    t.session.close();
    assert_eq!(outcome, Outcome::Complete);
    assert!(inner.join("made-here.txt").exists(), "the file belongs in the step's folder");
    assert!(!std::env::current_dir().unwrap().join("made-here.txt").exists());
}

/// Stands in for the elevation broker: records what it was asked to run.
struct RecordingBroker(Mutex<Vec<(String, String, String)>>);

impl keyjutsu_core::elevation::ElevatedRunner for RecordingBroker {
    fn run_step(
        &self,
        snapshot_hash: &str,
        step: &str,
        step_hash: &str,
    ) -> Result<keyjutsu_core::elevation::ElevatedRun, String> {
        self.0.lock().unwrap().push((snapshot_hash.into(), step.into(), step_hash.into()));
        Ok(keyjutsu_core::elevation::ElevatedRun {
            outcomes: vec![keyjutsu_core::execution::StepOutcome::Succeeded { exit_code: 0 }],
            output: "done elevated\n".into(),
        })
    }
}

#[test]
fn an_administrator_step_goes_to_the_broker_not_the_unelevated_shell() {
    if keyjutsu_core::elevation::is_elevated() {
        eprintln!("skipped: this test process is elevated, so no broker is needed");
        return;
    }
    let dir = scratch("admin-step");
    let marker = fwd(&dir.join("typed-here.txt"));
    let mut s = step("admin", &format!("Set-Content -LiteralPath {marker} -Value wrong-shell"));
    s["privilege"] = json!("administrator");
    let v = plan(json!([s]), json!([]));
    let draft = parse_plan(&v.to_string()).unwrap();
    let report = validate(&draft, Options { dry_run: false, broker_available: true });
    assert!(report.not_ready(&draft).is_empty(), "{:#?}", report.steps);
    let validated =
        keyjutsu_core::plan::ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    book.approve_all_except_critical(&validated, AT);
    let snap = seal(&validated, &book, None, AT).unwrap();

    // Without a broker it is refused, and nothing runs.
    let t = terminal();
    let (outcome, _, _) = run(&t, &snap, &mode(ExecutionMode::Direct), None);
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("needs Administrator")),
        "{outcome:?}"
    );

    // With one, the broker is asked for exactly this step of this snapshot.
    let broker = Arc::new(RecordingBroker(Mutex::default()));
    let options = ExecuteOptions { elevated_runner: Some(broker.clone()), ..mode(ExecutionMode::Direct) };
    let (outcome, _, events) = run(&t, &snap, &options, None);
    t.session.close();
    assert_eq!(outcome, Outcome::Complete, "{events:?}");
    assert_eq!(
        broker.0.lock().unwrap().as_slice(),
        [(snap.snapshot_hash().to_owned(), "admin".to_owned(), snap.step_hashes()["admin"].clone())]
    );
    assert!(
        events.iter().any(
            |e| matches!(e, ExecutionEvent::ElevatedOutput { text, .. } if text.contains("done elevated"))
        )
    );
    assert!(!dir.join("typed-here.txt").exists(), "it was never typed into the unelevated shell");
}
