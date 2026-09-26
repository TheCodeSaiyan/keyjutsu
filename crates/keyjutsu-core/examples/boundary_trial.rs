//! A real Windows restart across a plan boundary, for a
//! disposable machine such as Windows Sandbox. Never run it on a machine
//! whose restart would cost anything: the script around it restarts Windows.
//!
//! `boundary_trial phase1 DIR` runs phase 1 of a two-phase plan and stops at
//! the restart boundary, checks that resuming now is refused, and leaves the
//! snapshot and checkpoint in DIR. After the restart, `boundary_trial phase2
//! DIR` resumes: without a confirmation (refused), then with one. Everything
//! it sees goes to standard output for the log.

#![allow(clippy::unwrap_used, clippy::print_stdout)] // A trial program, not a library.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keyjutsu_core::boundary::{BoundaryNotice, ResumeGate};
use keyjutsu_core::execute::{Checkpoint, Driver, ExecuteOptions, ForwardingSink, Outcome, execute};
use keyjutsu_core::execution::ExecutionMode;
use keyjutsu_core::fingerprint;
use keyjutsu_core::headless::Collector;
use keyjutsu_core::plan::model::Readiness;
use keyjutsu_core::plan::{ApprovalBook, ApprovedSnapshot, ValidPlan, parse_plan, seal};
use keyjutsu_core::terminal::{ProfileMode, ShellKind};
use keyjutsu_core::validation::{Options, validate};
use keyjutsu_core::{Session, SessionOptions};

fn fwd(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

fn open_shell() -> (Session, std::sync::mpsc::Receiver<keyjutsu_core::SessionEvent>) {
    let (tx, events) = channel();
    let sink = Arc::new(ForwardingSink { inner: Arc::new(Collector::new()), events: Mutex::new(tx) });
    let mut o = SessionOptions::new(ShellKind::WindowsPowershell);
    o.profile = ProfileMode::Clean;
    o.intercept_cursor_queries = true;
    let session = Session::open(o, sink).unwrap();
    assert!(session.wait_ready(Duration::from_secs(120)), "Windows PowerShell never showed a prompt");
    (session, events)
}

fn options(gate: Option<ResumeGate>) -> ExecuteOptions {
    ExecuteOptions { mode: Some(ExecutionMode::Direct), resume_gate: gate, ..ExecuteOptions::default() }
}

fn phase1(dir: &Path) -> ExitCode {
    let marker = fwd(&dir.join("marker.txt"));
    let result = fwd(&dir.join("result.txt"));
    let text = serde_json::json!({
        "schema_version": "1.0", "plan_id": "restart-trial", "task_id": "t",
        "title": "Restart trial",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "phases": [
            {"id": "before-restart", "steps": ["prepare"], "boundary_after": "windows_restart"},
            {"id": "after-restart", "steps": ["finish"]}
        ],
        "steps": [
            {"id": "prepare", "title": "Prepare", "objective": "Leave a marker.", "kind": "command",
             "shell": {"kind": "windows_powershell"},
             "commands": [{"text": format!("Set-Content -LiteralPath {marker} -Value before-restart")}],
             "internal_validation": [{"path_exists": {"path": marker}}]},
            {"id": "finish", "title": "Finish", "objective": "Record completion.", "kind": "command",
             "shell": {"kind": "windows_powershell"}, "depends_on": ["prepare"],
             "commands": [{"text": format!("Set-Content -LiteralPath {result} -Value finished-after-restart")}]}
        ]
    })
    .to_string();
    let draft = parse_plan(&text).unwrap();
    let report = validate(&draft, Options { dry_run: false, ..Options::default() });
    for (id, s) in &report.steps {
        println!("validated {id}: {:?}", s.readiness);
    }
    if report.steps.values().any(|s| s.readiness != Readiness::Ready) {
        println!("FAIL: not every step is READY");
        return ExitCode::FAILURE;
    }
    let at = fingerprint::now_rfc3339();
    let v = ValidPlan::revalidate(report.record_in(draft.plan(), &at), false).unwrap();
    let mut book = ApprovalBook::new();
    book.approve_all_except_critical(&v, &at);
    let snap = seal(&v, &book, Some(fingerprint::collect(Some(v.plan()))), &at).unwrap();
    std::fs::write(dir.join("snapshot.json"), snap.to_json()).unwrap();

    let (session, events) = open_shell();
    let (outcome, checkpoint) = execute(
        &Driver { session: &session, events: &events },
        &snap,
        None,
        &options(None),
        &fingerprint::now_rfc3339,
        &|_| {},
    );
    println!("phase 1 outcome: {outcome:?}");
    println!("recorded boot identity: {:?}", checkpoint.boundary.as_ref().and_then(|b| b.identity.clone()));
    std::fs::write(dir.join("checkpoint.json"), serde_json::to_string_pretty(&checkpoint).unwrap()).unwrap();
    if !matches!(outcome, Outcome::Boundary { .. })
        || checkpoint.boundary.as_ref().and_then(|b| b.identity.as_ref()).is_none()
    {
        println!("FAIL: expected to stop at the boundary with a boot identity");
        return ExitCode::FAILURE;
    }

    // Before any restart, resuming must be refused.
    let yes: ResumeGate = Arc::new(|_| true);
    let (early, _) = execute(
        &Driver { session: &session, events: &events },
        &snap,
        Some(checkpoint),
        &options(Some(yes)),
        &fingerprint::now_rfc3339,
        &|_| {},
    );
    session.close();
    println!("resume before restart: {early:?}");
    if !matches!(&early, Outcome::Blocked { reason } if reason.contains("has not happened")) {
        println!("FAIL: resuming before the restart was not refused");
        return ExitCode::FAILURE;
    }
    println!("PASS phase 1: stopped at the boundary; early resume refused; ready for a real restart");
    ExitCode::SUCCESS
}

fn phase2(dir: &Path) -> ExitCode {
    let snap =
        ApprovedSnapshot::from_json(&std::fs::read_to_string(dir.join("snapshot.json")).unwrap()).unwrap();
    let checkpoint: Checkpoint =
        serde_json::from_str(&std::fs::read_to_string(dir.join("checkpoint.json")).unwrap()).unwrap();
    println!("waiting at: {:?}", checkpoint.boundary);
    let (session, events) = open_shell();

    // Restarted, but not yet confirmed: nothing may run.
    let (unconfirmed, _) = execute(
        &Driver { session: &session, events: &events },
        &snap,
        Some(checkpoint.clone()),
        &options(None),
        &fingerprint::now_rfc3339,
        &|_| {},
    );
    println!("resume without confirmation: {unconfirmed:?}");
    let refused = matches!(&unconfirmed, Outcome::Blocked { reason } if reason.contains("confirmation"));

    let seen: Arc<Mutex<Option<BoundaryNotice>>> = Arc::default();
    let s = seen.clone();
    let gate: ResumeGate = Arc::new(move |n: &BoundaryNotice| {
        *s.lock().unwrap() = Some(n.clone());
        true
    });
    let (outcome, _) = execute(
        &Driver { session: &session, events: &events },
        &snap,
        Some(checkpoint),
        &options(Some(gate)),
        &fingerprint::now_rfc3339,
        &|_| {},
    );
    session.close();
    println!("resume confirmed: {outcome:?}");
    let notice = seen.lock().unwrap().clone();
    println!("notice shown to the operator: {notice:?}");
    let result = std::fs::read_to_string(dir.join("result.txt")).unwrap_or_default();
    println!("result.txt: {}", result.trim());
    let verified = notice.as_ref().is_some_and(|n| n.verified);
    if refused && outcome == Outcome::Complete && verified && result.trim() == "finished-after-restart" {
        println!(
            "PASS phase 2: the restart was seen, the machine rechecked, the operator asked, phase 2 ran"
        );
        ExitCode::SUCCESS
    } else {
        println!("FAIL: refused={refused} outcome={outcome:?} verified={verified}");
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match (args.get(1).map(String::as_str), args.get(2)) {
        (Some("phase1"), Some(dir)) => phase1(&PathBuf::from(dir)),
        (Some("phase2"), Some(dir)) => phase2(&PathBuf::from(dir)),
        _ => {
            println!("usage: boundary_trial phase1|phase2 DIR");
            ExitCode::from(2)
        }
    }
}
