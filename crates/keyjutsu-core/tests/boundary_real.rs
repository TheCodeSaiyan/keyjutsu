//! Milestone 15: a plan in phases stops at a session boundary, and resumes
//! only once the boundary has really happened, the machine still matches,
//! what the first phase achieved still holds, and the operator says so.
//!
//! The shell restart is real: the second half runs in a new shell. A Windows
//! restart cannot be done in a test, so its boot identity is injected; the
//! same code checks it.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keyjutsu_core::boundary::{BoundaryNotice, BoundaryProbe, FingerprintNow, ResumeGate};
use keyjutsu_core::execute::{Checkpoint, Driver, ExecuteOptions, ForwardingSink, Outcome, execute};
use keyjutsu_core::execution::ExecutionMode;
use keyjutsu_core::fingerprint;
use keyjutsu_core::headless::Collector;
use keyjutsu_core::plan::model::Boundary;
use keyjutsu_core::plan::{ApprovalBook, ApprovedSnapshot, ValidPlan, parse_plan, seal};
use keyjutsu_core::terminal::{ProfileMode, ShellKind};
use keyjutsu_core::validation::{Options, validate};
use keyjutsu_core::{Session, SessionEvent, SessionOptions};
use serde_json::json;

const AT: &str = "2026-09-25T10:00:00Z";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("boundary").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fwd(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Phase 1 makes a file and sets a shell variable; the boundary; phase 2
/// reports whether the variable survived and writes a result file.
fn phased(dir: &Path, boundary: &str) -> ApprovedSnapshot {
    let marker = fwd(&dir.join("marker.txt"));
    let result = fwd(&dir.join("result.txt"));
    let draft = parse_plan(
        &json!({
            "schema_version": "1.0", "plan_id": "p", "task_id": "t",
            "target": {"id": "local", "kind": "local_windows"},
            "agent": {"name": "codex", "version": "1"},
            "phases": [
                {"id": "before", "steps": ["prepare"], "boundary_after": boundary},
                {"id": "after", "steps": ["finish"]}
            ],
            "steps": [
                {"id": "prepare", "title": "Prepare", "objective": "Leave a marker.", "kind": "command",
                 "shell": {"kind": "pwsh"},
                 "commands": [{"text": format!("Set-Content -LiteralPath {marker} -Value made; $global:KJ_BEFORE = 'still here'")}],
                 "internal_validation": [{"path_exists": {"path": marker}}]},
                {"id": "finish", "title": "Finish", "objective": "Use the marker.", "kind": "command",
                 "shell": {"kind": "pwsh"}, "depends_on": ["prepare"],
                 "commands": [{"text": format!("Set-Content -LiteralPath {result} -Value ('shell variable: ' + [bool]$global:KJ_BEFORE)")}]}
            ]
        })
        .to_string(),
    )
    .unwrap();
    let report = validate(&draft, Options { dry_run: false, ..Options::default() });
    assert!(report.not_ready(&draft).is_empty(), "{:#?}", report.steps);
    let v = ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    assert!(book.approve_all_except_critical(&v, AT).is_empty());
    seal(&v, &book, Some(fingerprint::collect(Some(v.plan()))), AT).unwrap()
}

struct Shell {
    session: Session,
    events: std::sync::mpsc::Receiver<SessionEvent>,
}

fn shell() -> Shell {
    let (tx, events) = channel();
    let sink = Arc::new(ForwardingSink { inner: Arc::new(Collector::new()), events: Mutex::new(tx) });
    let mut o = SessionOptions::new(ShellKind::Pwsh);
    o.profile = ProfileMode::Clean;
    o.intercept_cursor_queries = true;
    let session = Session::open(o, sink).unwrap();
    assert!(session.wait_ready(Duration::from_secs(30)));
    Shell { session, events }
}

fn run(
    s: &Shell,
    snap: &ApprovedSnapshot,
    options: &ExecuteOptions,
    resume: Option<Checkpoint>,
) -> (Outcome, Checkpoint) {
    execute(
        &Driver { session: &s.session, events: &s.events },
        snap,
        resume,
        options,
        &|| AT.to_owned(),
        &|_| {},
    )
}

fn options(gate: Option<ResumeGate>) -> ExecuteOptions {
    ExecuteOptions { mode: Some(ExecutionMode::Direct), resume_gate: gate, ..ExecuteOptions::default() }
}

fn yes(asked: Arc<Mutex<Vec<BoundaryNotice>>>) -> ResumeGate {
    Arc::new(move |n: &BoundaryNotice| {
        asked.lock().unwrap().push(n.clone());
        true
    })
}

#[test]
fn a_plan_crosses_a_real_shell_restart_without_trusting_the_old_shell() {
    let dir = scratch("shell-restart");
    let snap = phased(&dir, "shell_restart");
    let asked: Arc<Mutex<Vec<BoundaryNotice>>> = Arc::default();

    // Phase 1, then a stop at the boundary.
    let first = shell();
    let (outcome, checkpoint) = run(&first, &snap, &options(Some(yes(asked.clone()))), None);
    assert_eq!(outcome, Outcome::Boundary { phase: "before".into(), boundary: Boundary::ShellRestart });
    assert!(dir.join("marker.txt").exists() && !dir.join("result.txt").exists(), "phase 2 did not start");
    let wait = checkpoint.boundary.clone().unwrap();
    assert!(wait.identity.is_some(), "the old shell is recorded");

    // Resuming in the same shell is refused: the restart has not happened.
    let (outcome, _) = run(&first, &snap, &options(Some(yes(asked.clone()))), Some(checkpoint.clone()));
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("has not happened")),
        "{outcome:?}"
    );
    first.session.close();

    // A new shell: checked, confirmed, phase 2 runs; nothing from the old
    // shell survived, and nothing depended on it.
    let second = shell();
    let (outcome, done) = run(&second, &snap, &options(Some(yes(asked.clone()))), Some(checkpoint));
    second.session.close();
    assert_eq!(outcome, Outcome::Complete);
    assert_eq!(std::fs::read_to_string(dir.join("result.txt")).unwrap().trim(), "shell variable: False");
    assert!(done.boundary.is_none());
    let notices = asked.lock().unwrap();
    assert_eq!(notices.len(), 1, "asked once, after the checks passed");
    assert!(notices[0].verified);
    assert!(notices[0].rechecked.iter().any(|c| c.check.starts_with("prepare:") && c.passed == Some(true)));
}

#[test]
fn what_phase_one_achieved_is_checked_again_after_the_boundary() {
    let dir = scratch("undone");
    let snap = phased(&dir, "shell_restart");
    let first = shell();
    let (_, checkpoint) = run(&first, &snap, &options(None), None);
    first.session.close();

    // Something removed the marker while the shell was down.
    std::fs::remove_file(dir.join("marker.txt")).unwrap();
    let second = shell();
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    let gate: ResumeGate = Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        true
    });
    let (outcome, kept) = run(&second, &snap, &options(Some(gate)), Some(checkpoint));
    second.session.close();
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("no longer holds")),
        "{outcome:?}"
    );
    assert!(!dir.join("result.txt").exists(), "phase 2 did not run on a broken assumption");
    assert_eq!(asked.load(Ordering::SeqCst), 0, "the operator is not asked to approve a broken state");
    assert!(kept.boundary.is_some(), "still waiting at the boundary");
}

#[test]
fn a_windows_restart_is_required_confirmed_and_followed_by_a_fresh_look_at_the_machine() {
    let dir = scratch("windows");
    let snap = phased(&dir, "windows_restart");
    let boot = Arc::new(Mutex::new("boot-1".to_owned()));
    let b = boot.clone();
    let probe: BoundaryProbe = Arc::new(move |_, _| Some(b.lock().unwrap().clone()));
    let with = |gate: Option<ResumeGate>, fp: Option<FingerprintNow>| ExecuteOptions {
        boundary_probe: Some(probe.clone()),
        fingerprint_now: fp,
        ..options(gate)
    };

    let s = shell();
    let (outcome, checkpoint) = run(&s, &snap, &with(None, None), None);
    assert_eq!(outcome, Outcome::Boundary { phase: "before".into(), boundary: Boundary::WindowsRestart });
    assert_eq!(checkpoint.boundary.as_ref().unwrap().identity.as_deref(), Some("boot-1"));

    // Not restarted yet.
    let (outcome, _) = run(&s, &snap, &with(Some(Arc::new(|_| true)), None), Some(checkpoint.clone()));
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("has not happened")),
        "{outcome:?}"
    );

    *boot.lock().unwrap() = "boot-2".into();

    // Restarted, but the restart installed a different PowerShell.
    let mut changed = fingerprint::collect(Some(snap.plan()));
    for sh in &mut changed.shells {
        if sh.name == "pwsh" {
            sh.version = Some("7.0.0".into());
        }
    }
    let fp: FingerprintNow = Arc::new(move |_| changed.clone());
    let (outcome, _) = run(&s, &snap, &with(Some(Arc::new(|_| true)), Some(fp)), Some(checkpoint.clone()));
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("require revalidation") && reason.contains("finish")),
        "{outcome:?}"
    );

    // Restarted, unchanged, but without the operator's word: nothing runs.
    let (outcome, _) = run(&s, &snap, &with(None, None), Some(checkpoint.clone()));
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("confirmation")),
        "{outcome:?}"
    );
    let (outcome, _) = run(&s, &snap, &with(Some(Arc::new(|_| false)), None), Some(checkpoint.clone()));
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("did not confirm")),
        "{outcome:?}"
    );
    assert!(!dir.join("result.txt").exists());

    // Restarted, unchanged, confirmed: phase 2 runs.
    let (outcome, _) = run(&s, &snap, &with(Some(Arc::new(|_| true)), None), Some(checkpoint));
    s.session.close();
    assert_eq!(outcome, Outcome::Complete);
    assert!(dir.join("result.txt").exists());
}
