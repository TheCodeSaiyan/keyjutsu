//! Milestone 12: a task that depends on a download is staged before it is
//! armed and runs the verified staged copy. The "internet" is a small HTTP
//! server on 127.0.0.1 whose content the test changes under the plan.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keyjutsu_core::artifacts::{pin, stage, verify};
use keyjutsu_core::execute::{Driver, ExecuteOptions, ForwardingSink, Outcome, execute};
use keyjutsu_core::execution::ExecutionMode;
use keyjutsu_core::headless::Collector;
use keyjutsu_core::plan::model::{Plan, Readiness};
use keyjutsu_core::plan::{ApprovalBook, ApprovedSnapshot, ValidPlan, parse_plan, seal};
use keyjutsu_core::terminal::{ProfileMode, ShellKind};
use keyjutsu_core::validation::{Options, validate};
use keyjutsu_core::{Session, SessionOptions};
use serde_json::json;

const AT: &str = "2026-09-25T08:00:00Z";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("artifacts").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fwd(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Serves `body` to anyone who asks, counting requests.
struct Server {
    port: u16,
    body: Arc<Mutex<Vec<u8>>>,
    requests: Arc<AtomicUsize>,
}

fn serve(body: &str) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let shared = Arc::new(Mutex::new(body.as_bytes().to_vec()));
    let requests = Arc::new(AtomicUsize::new(0));
    let (b, r) = (shared.clone(), requests.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 4096];
            let mut seen = Vec::new();
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => seen.extend_from_slice(&buf[..n]),
                }
            }
            r.fetch_add(1, Ordering::SeqCst);
            let body = b.lock().unwrap().clone();
            let head =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
        }
    });
    Server { port, body: shared, requests }
}

fn plan_for(server: &Server, dest: &Path) -> Plan {
    serde_json::from_value(json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [{
            "id": "install", "title": "Install the tool", "objective": "Put the staged tool in place.",
            "kind": "command", "shell": {"kind": "pwsh"},
            "commands": [{"text": format!("Copy-Item -LiteralPath $KJ_ARTIFACTS['tool.txt'] -Destination {}", fwd(dest))}],
            "network": {"destinations": [{"host": "127.0.0.1", "protocol": "http", "purpose": "the tool", "at_runtime": false}]},
            "artifacts": [{"name": "tool.txt", "source": format!("http://127.0.0.1:{}/tool.txt", server.port), "version": "1.0"}]
        }]
    }))
    .unwrap()
}

fn validated(plan: &Plan) -> (ValidPlan, BTreeMapReadiness) {
    let draft = parse_plan(&serde_json::to_string(plan).unwrap()).unwrap();
    let report = validate(&draft, Options { dry_run: false, ..Options::default() });
    let readiness = report.steps.iter().map(|(k, v)| (k.clone(), v.readiness)).collect();
    (ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap(), readiness)
}

type BTreeMapReadiness = std::collections::BTreeMap<String, Readiness>;

fn approve(plan: &Plan) -> ApprovedSnapshot {
    let (v, readiness) = validated(plan);
    assert!(readiness.values().all(|r| *r == Readiness::Ready), "{readiness:?}");
    let mut book = ApprovalBook::new();
    assert!(book.approve_all_except_critical(&v, AT).is_empty());
    seal(&v, &book, None, AT).unwrap()
}

fn run(snap: &ApprovedSnapshot, store: &Path) -> Outcome {
    let (tx, events) = channel();
    let sink = Arc::new(ForwardingSink { inner: Arc::new(Collector::new()), events: Mutex::new(tx) });
    let mut o = SessionOptions::new(ShellKind::Pwsh);
    o.profile = ProfileMode::Clean;
    o.intercept_cursor_queries = true;
    let session = Session::open(o, sink).unwrap();
    assert!(session.wait_ready(Duration::from_secs(30)));
    let options = ExecuteOptions {
        mode: Some(ExecutionMode::Direct),
        artifact_store: store.to_owned(),
        ..ExecuteOptions::default()
    };
    let (outcome, _) = execute(
        &Driver { session: &session, events: &events },
        snap,
        None,
        &options,
        &|| AT.to_owned(),
        &|_| {},
    );
    session.close();
    outcome
}

#[test]
fn a_download_dependent_task_runs_the_verified_staged_copy() {
    let dir = scratch("done-when");
    let store = dir.join("store");
    let dest = dir.join("installed.txt");
    let server = serve("tool version 1\n");
    let unpinned = plan_for(&server, &dest);

    // Unpinned, it cannot be approved: a moving target is not approvable.
    let (_, readiness) = validated(&unpinned);
    assert_eq!(readiness["install"], Readiness::NeedsReview);

    // Stage: download, hash, keep, record provenance; then pin the hash.
    let a = &unpinned.steps[0].artifacts[0];
    let staged = stage(&store, a, AT).unwrap();
    assert!(!staged.was_pinned);
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read_to_string(&staged.path).unwrap(), "tool version 1\n");
    let pinned = pin(&unpinned, std::slice::from_ref(&staged));
    assert_eq!(pinned.steps[0].artifacts[0].sha256.as_deref(), Some(staged.sha256.as_str()));
    let meta = PathBuf::from(&staged.path).with_file_name("provenance.json");
    assert!(std::fs::read_to_string(meta).unwrap().contains(&a.source), "where it came from is kept");

    let snap = approve(&pinned);

    // The source moves on. The run must not notice.
    *server.body.lock().unwrap() = b"tool version 2, not approved\n".to_vec();
    server.requests.store(0, Ordering::SeqCst);
    assert_eq!(run(&snap, &store), Outcome::Complete);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "tool version 1\n", "the staged, approved copy ran");
    assert_eq!(server.requests.load(Ordering::SeqCst), 0, "nothing was downloaded while it ran");
}

#[test]
fn a_staged_copy_that_changed_stops_the_plan() {
    let dir = scratch("tampered");
    let store = dir.join("store");
    let dest = dir.join("installed.txt");
    let server = serve("good\n");
    let unpinned = plan_for(&server, &dest);
    let staged = stage(&store, &unpinned.steps[0].artifacts[0], AT).unwrap();
    let snap = approve(&pin(&unpinned, std::slice::from_ref(&staged)));

    std::fs::write(&staged.path, "evil\n").unwrap();
    let outcome = run(&snap, &store);
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("changed since it was staged")),
        "{outcome:?}"
    );
    assert!(!dest.exists(), "nothing ran");
}

#[test]
fn a_plan_that_was_never_staged_does_not_arm() {
    let dir = scratch("unstaged");
    let dest = dir.join("installed.txt");
    let server = serve("x\n");
    let mut p = plan_for(&server, &dest);
    p.steps[0].artifacts[0].sha256 = Some(keyjutsu_core::plan::hash::sha256_hex(b"x\n"));
    let snap = approve(&p);
    let outcome = run(&snap, &dir.join("empty-store"));
    assert!(matches!(&outcome, Outcome::Blocked { reason } if reason.contains("not staged")), "{outcome:?}");
    assert_eq!(server.requests.load(Ordering::SeqCst), 0, "arming never downloads");
}

#[test]
fn a_download_that_does_not_match_its_pin_is_not_kept() {
    let dir = scratch("mismatch");
    let store = dir.join("store");
    let server = serve("what the server has now\n");
    let mut p = plan_for(&server, &dir.join("x"));
    let expected = keyjutsu_core::plan::hash::sha256_hex(b"what the plan approved\n");
    p.steps[0].artifacts[0].sha256 = Some(expected.clone());
    let err = stage(&store, &p.steps[0].artifacts[0], AT).unwrap_err();
    assert!(err.contains("pins"), "{err}");
    assert!(verify(&store, &p.steps[0].artifacts[0]).is_err());
    let left: Vec<_> = std::fs::read_dir(&store).unwrap().flatten().collect();
    assert!(left.is_empty(), "no partial or wrong copy is kept: {left:?}");
}

#[test]
fn staging_in_the_workspace_pins_the_hash_and_asks_for_validation_again() {
    use keyjutsu_core::workspace::Workspace;
    let dir = scratch("workspace");
    let server = serve("from the workspace\n");
    let p = plan_for(&server, &dir.join("out.txt"));
    let mut w = Workspace::open(&serde_json::to_string(&p).unwrap()).unwrap();
    w.validate(Options { dry_run: false, ..Options::default() }, AT).unwrap();
    assert_eq!(w.view().steps[0].readiness, Some(Readiness::NeedsReview), "unpinned");

    let staged = w.stage(&dir.join("store"), AT).unwrap();
    assert_eq!(w.plan().steps[0].artifacts[0].sha256.as_deref(), Some(staged[0].sha256.as_str()));
    assert_eq!(w.view().steps[0].readiness, None, "pinning is an edit: validate again");
    w.validate(Options { dry_run: false, ..Options::default() }, AT).unwrap();
    assert_eq!(w.view().steps[0].readiness, Some(Readiness::Ready));
}
