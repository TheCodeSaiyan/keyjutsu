//! A failed reversible task, recovered under the operator's
//! control, on the real file system and registry. Registry work happens under
//! a key of its own in HKCU, which each test removes when it finishes.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keyjutsu_core::execute::{Checkpoint, Driver, ExecuteOptions, ForwardingSink, Outcome, execute};
use keyjutsu_core::execution::{ExecutionMode, PerformanceConfig};
use keyjutsu_core::headless::Collector;
use keyjutsu_core::plan::{ApprovalBook, ApprovedSnapshot, parse_plan, seal};
use keyjutsu_core::recovery::{RecoveryItem, plan_recovery, recover, recovery_dir};
use keyjutsu_core::terminal::{ProfileMode, ShellKind};
use keyjutsu_core::validation::{Options, validate};
use keyjutsu_core::{Session, SessionOptions};
use serde_json::{Value, json};

const AT: &str = "2026-09-25T05:00:00Z";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("recovery").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fwd(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

fn pwsh(script: &str) -> String {
    let out = std::process::Command::new("pwsh")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A registry key of the test's own, removed when dropped.
struct TestKey(String);

impl TestKey {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let key = format!("HKCU:\\Software\\KeyJutsu-Tests\\{name}-{}-{nanos}", std::process::id());
        pwsh(&format!("New-Item -Path '{key}' -Force | Out-Null"));
        Self(key)
    }

    fn value(&self, name: &str) -> String {
        pwsh(&format!(
            "$k = Get-Item -LiteralPath '{}'; if ($k.GetValueNames() -contains '{name}') {{ $k.GetValueKind('{name}').ToString() + ':' + (($k.GetValue('{name}')) -join ',') }} else {{ 'absent' }}",
            self.0
        ))
    }
}

impl Drop for TestKey {
    fn drop(&mut self) {
        let _ = std::process::Command::new("pwsh")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            // The shared parent goes too once it is empty. Without -Recurse,
            // removing it fails while another test's key is still inside.
            .arg(format!(
                "Remove-Item -LiteralPath '{}' -Recurse -Force; $p = 'HKCU:\\Software\\KeyJutsu-Tests'; if (-not (Get-ChildItem -LiteralPath $p)) {{ Remove-Item -LiteralPath $p -ErrorAction SilentlyContinue }}",
                self.0
            ))
            .output();
    }
}

fn step(id: &str, command: &str) -> Value {
    json!({"id": id, "title": id, "objective": "Test.", "kind": "command", "shell": {"kind": "pwsh"},
           "commands": [{"text": command}]})
}

fn reversible(id: &str, command: &str, captures: Value) -> Value {
    let mut s = step(id, command);
    s["reversibility"] = json!({"level": "full"});
    s["recovery"] = json!({"strategy": "restore_captured_state", "capture": captures});
    s
}

fn plan(steps: Value) -> Value {
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": steps
    })
}

fn approve(v: &Value) -> ApprovedSnapshot {
    let draft = parse_plan(&v.to_string()).unwrap();
    let report = validate(&draft, Options { dry_run: false, ..Options::default() });
    assert!(report.not_ready(&draft).is_empty(), "{:#?}", report.steps);
    let validated =
        keyjutsu_core::plan::ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    assert!(book.approve_all_except_critical(&validated, AT).is_empty());
    seal(&validated, &book, None, AT).unwrap()
}

struct Terminal {
    session: Session,
    events: std::sync::mpsc::Receiver<keyjutsu_core::SessionEvent>,
}

fn terminal() -> Terminal {
    let (tx, events) = channel();
    let sink = Arc::new(ForwardingSink { inner: Arc::new(Collector::new()), events: Mutex::new(tx) });
    let mut o = SessionOptions::new(ShellKind::Pwsh);
    o.profile = ProfileMode::Clean;
    o.intercept_cursor_queries = true;
    let session = Session::open(o, sink).unwrap();
    assert!(session.wait_ready(Duration::from_secs(30)));
    Terminal { session, events }
}

fn run(t: &Terminal, snap: &ApprovedSnapshot, checkpoint: &Path) -> (Outcome, Checkpoint) {
    let options = ExecuteOptions {
        mode: Some(ExecutionMode::Direct),
        checkpoint: Some(checkpoint.to_owned()),
        ..ExecuteOptions::default()
    };
    execute(
        &Driver { session: &t.session, events: &t.events },
        snap,
        None,
        &options,
        &|| AT.to_owned(),
        &|_| {},
    )
}

fn modified(p: &Path) -> std::time::SystemTime {
    std::fs::metadata(p).unwrap().modified().unwrap()
}

#[test]
fn a_failed_reversible_task_is_recovered_without_touching_anything_else() {
    let dir = scratch("done-when");
    let key = TestKey::new("done-when");
    let (target, created, unrelated) =
        (dir.join("target.txt"), dir.join("created.txt"), dir.join("unrelated.txt"));
    std::fs::write(&target, "original\r\n").unwrap();
    std::fs::write(&unrelated, "leave me alone").unwrap();
    pwsh(&format!(
        "New-ItemProperty -LiteralPath '{0}' -Name Setting -Value old -PropertyType String | Out-Null; New-ItemProperty -LiteralPath '{0}' -Name Other -Value 7 -PropertyType DWord | Out-Null",
        key.0
    ));
    let unrelated_since = modified(&unrelated);
    let setting = format!("{}\\Setting", key.0);
    let added = format!("{}\\Added", key.0);

    let mut verify = step("verify", "Get-Date | Out-Null");
    verify["internal_validation"] = json!([{"path_exists": {"path": fwd(&dir.join("never.txt"))}}]);
    let snap = approve(&plan(json!([
        reversible(
            "edit-file",
            &format!("Set-Content -LiteralPath {} -Value changed", fwd(&target)),
            json!([{"kind": "file", "target": fwd(&target)}])
        ),
        reversible(
            "edit-registry",
            &format!("Set-ItemProperty -LiteralPath '{}' -Name Setting -Value new", key.0),
            json!([{"kind": "registry_value", "target": setting}])
        ),
        reversible(
            "new-file",
            &format!("Set-Content -LiteralPath {} -Value fresh", fwd(&created)),
            json!([{"kind": "file", "target": fwd(&created)}])
        ),
        reversible(
            "new-registry",
            &format!(
                "New-ItemProperty -LiteralPath '{}' -Name Added -Value 1 -PropertyType DWord | Out-Null",
                key.0
            ),
            json!([{"kind": "registry_value", "target": added}])
        ),
        verify
    ])));
    let cp = dir.join("run.checkpoint.json");
    let t = terminal();
    let (outcome, checkpoint) = run(&t, &snap, &cp);
    t.session.close();
    assert!(matches!(&outcome, Outcome::Failed { step, .. } if step == "verify"), "{outcome:?}");

    // The task really did change things.
    assert_eq!(std::fs::read_to_string(&target).unwrap().trim(), "changed");
    assert!(created.exists());
    assert_eq!(key.value("Setting"), "String:new");
    assert_eq!(key.value("Added"), "DWord:1");

    // Nothing is undone until the operator reviews and confirms.
    let saved = Checkpoint::load(&cp).unwrap();
    assert_eq!(saved.captures.len(), 4);
    let items = plan_recovery(&snap, &saved, &[]).unwrap();
    let order: Vec<&str> = items.iter().map(RecoveryItem::step).collect();
    assert_eq!(order, ["verify", "new-registry", "new-file", "edit-registry", "edit-file"], "latest first");
    assert!(matches!(&items[0], RecoveryItem::Cannot { .. }), "the failing step declared no recovery");

    let results = recover(
        None,
        None,
        &snap,
        &saved,
        &recovery_dir(&cp),
        &items,
        &PerformanceConfig::default(),
        &|_| {},
    );
    assert_eq!(results.len(), 4);
    assert!(results.iter().all(|r| r.recovered), "{results:#?}");
    drop(checkpoint);

    // Everything the task changed is as it was...
    assert_eq!(std::fs::read(&target).unwrap(), b"original\r\n");
    assert!(!created.exists());
    assert_eq!(key.value("Setting"), "String:old");
    assert_eq!(key.value("Added"), "absent");
    // ...and what it never declared was not touched.
    assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "leave me alone");
    assert_eq!(modified(&unrelated), unrelated_since);
    assert_eq!(key.value("Other"), "DWord:7");
}

#[test]
fn only_the_steps_the_operator_chooses_are_recovered() {
    let dir = scratch("only");
    let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
    std::fs::write(&a, "a0").unwrap();
    std::fs::write(&b, "b0").unwrap();
    let snap = approve(&plan(json!([
        reversible(
            "a",
            &format!("Set-Content -LiteralPath {} -Value a1", fwd(&a)),
            json!([{"kind": "file", "target": fwd(&a)}])
        ),
        reversible(
            "b",
            &format!("Set-Content -LiteralPath {} -Value b1", fwd(&b)),
            json!([{"kind": "file", "target": fwd(&b)}])
        ),
    ])));
    let cp = dir.join("run.checkpoint.json");
    let t = terminal();
    let (outcome, checkpoint) = run(&t, &snap, &cp);
    t.session.close();
    assert_eq!(outcome, Outcome::Complete);

    let items = plan_recovery(&snap, &checkpoint, &["b".into()]).unwrap();
    recover(
        None,
        None,
        &snap,
        &checkpoint,
        &recovery_dir(&cp),
        &items,
        &PerformanceConfig::default(),
        &|_| {},
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap().trim(), "a1", "not chosen, not touched");
    assert_eq!(std::fs::read(&b).unwrap(), b"b0");
    assert!(plan_recovery(&snap, &checkpoint, &["nope".into()]).is_err());
}

#[test]
fn a_backup_changed_since_it_was_taken_is_not_used() {
    let dir = scratch("tampered");
    let f = dir.join("f.txt");
    std::fs::write(&f, "before").unwrap();
    let snap = approve(&plan(json!([reversible(
        "edit",
        &format!("Set-Content -LiteralPath {} -Value after", fwd(&f)),
        json!([{"kind": "file", "target": fwd(&f)}])
    ),])));
    let cp = dir.join("run.checkpoint.json");
    let t = terminal();
    let (_, checkpoint) = run(&t, &snap, &cp);
    t.session.close();

    std::fs::write(recovery_dir(&cp).join("edit-0.bak"), "something else").unwrap();
    let items = plan_recovery(&snap, &checkpoint, &[]).unwrap();
    let results = recover(
        None,
        None,
        &snap,
        &checkpoint,
        &recovery_dir(&cp),
        &items,
        &PerformanceConfig::default(),
        &|_| {},
    );
    assert!(!results[0].recovered);
    assert!(results[0].checks[0].detail.contains("changed since it was taken"), "{results:?}");
    assert_eq!(std::fs::read_to_string(&f).unwrap().trim(), "after", "the file was left as it is");
}

#[test]
fn a_step_whose_recovery_cannot_be_prepared_does_not_run() {
    let dir = scratch("unprepared");
    let marker = dir.join("ran.txt");
    let later = dir.join("later");
    let snap = approve(&plan(json!([reversible(
        "edit",
        &format!("Set-Content -LiteralPath {} -Value ran", fwd(&marker)),
        json!([{"kind": "file", "target": fwd(&later)}])
    ),])));
    // Validation refuses a folder capture, so the folder appears only after
    // approval: the run itself must still refuse to go ahead.
    std::fs::create_dir_all(&later).unwrap();
    let t = terminal();
    let (outcome, _) = run(&t, &snap, &dir.join("run.checkpoint.json"));
    t.session.close();
    assert!(
        matches!(&outcome, Outcome::Blocked { reason } if reason.contains("could not prepare recovery")),
        "{outcome:?}"
    );
    assert!(!marker.exists(), "the step ran without its recovery");
}

#[test]
fn recovery_commands_run_in_the_terminal_and_are_validated() {
    let dir = scratch("commands");
    let marker = dir.join("marker.txt");
    let mut s = step("make", &format!("Set-Content -LiteralPath {} -Value made", fwd(&marker)));
    s["reversibility"] = json!({"level": "full"});
    s["recovery"] = json!({"strategy": "commands",
        "commands": [{"text": format!("Remove-Item -LiteralPath {}", fwd(&marker))}],
        "validation": [{"exit_code": {"equals": 0}}]});
    let snap = approve(&plan(json!([s])));
    let cp = dir.join("run.checkpoint.json");
    let t = terminal();
    let (_, checkpoint) = run(&t, &snap, &cp);
    assert!(marker.exists());
    t.session.disarm();

    let items = plan_recovery(&snap, &checkpoint, &[]).unwrap();
    assert!(matches!(&items[0], RecoveryItem::Commands { .. }));
    let driver = Driver { session: &t.session, events: &t.events };
    let results = recover(
        Some(&driver),
        None,
        &snap,
        &checkpoint,
        &recovery_dir(&cp),
        &items,
        &PerformanceConfig::default(),
        &|_| {},
    );
    t.session.close();
    assert!(results[0].recovered, "{results:?}");
    assert!(!marker.exists());
}

/// Approved as a machine with the elevation broker installed approves it.
fn approve_with_broker(v: &Value) -> ApprovedSnapshot {
    let draft = parse_plan(&v.to_string()).unwrap();
    let report = validate(&draft, Options { dry_run: false, broker_available: true });
    assert!(report.not_ready(&draft).is_empty(), "{:#?}", report.steps);
    let validated =
        keyjutsu_core::plan::ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    assert!(book.approve_all_except_critical(&validated, AT).is_empty());
    seal(&validated, &book, None, AT).unwrap()
}

/// Records what it was asked to recover, and runs nothing.
#[derive(Default)]
struct RecordingBroker(Mutex<Vec<(String, String)>>);

impl keyjutsu_core::elevation::ElevatedRunner for RecordingBroker {
    fn run_step(
        &self,
        _: &str,
        step: &str,
        _: &str,
    ) -> Result<keyjutsu_core::elevation::ElevatedRun, String> {
        Err(format!("not asked to run `{step}`"))
    }
    fn recover_step(
        &self,
        _: &str,
        step: &str,
        step_hash: &str,
    ) -> Result<keyjutsu_core::elevation::ElevatedRun, String> {
        self.0.lock().unwrap().push((step.to_owned(), step_hash.to_owned()));
        Ok(keyjutsu_core::elevation::ElevatedRun {
            outcomes: vec![keyjutsu_core::execution::StepOutcome::Succeeded { exit_code: 0 }],
            output: String::new(),
        })
    }
}

fn ran(snap: &ApprovedSnapshot, ids: &[&str]) -> Checkpoint {
    let mut cp = Checkpoint::new(snap.snapshot_hash());
    for id in ids {
        cp.runs.push(keyjutsu_core::execute::StepRun {
            step: (*id).into(),
            step_hash: snap.step_hashes()[*id].clone(),
            succeeded: true,
            exit_code: Some(0),
            checks: Vec::new(),
            started_at: AT.into(),
            finished_at: AT.into(),
        });
    }
    cp
}

/// An Administrator step's recovery commands go to the elevation broker,
/// named by step and hash only; they are never typed into the unelevated
/// terminal. What it captured is not handed to the broker to write back:
/// that lives in the operator's profile.
#[test]
fn an_administrator_step_is_recovered_through_the_broker_and_never_from_the_profile() {
    if keyjutsu_core::elevation::is_elevated() {
        return; // Elevated, KeyJutsu recovers directly: nothing to show here.
    }
    let dir = scratch("admin");
    let mut undo = step("undo", "Get-Date");
    undo["privilege"] = json!("administrator");
    undo["reversibility"] = json!({"level": "full"});
    undo["recovery"] = json!({"strategy": "commands", "commands": [{"text": "Get-Location"}]});
    let mut restore = reversible(
        "restore",
        "Get-Date",
        json!([{"kind": "registry_value", "target": r"HKLM:\SOFTWARE\KeyJutsu-Tests\Value"}]),
    );
    restore["privilege"] = json!("administrator");
    let snap = approve_with_broker(&plan(json!([undo, restore])));
    let mut checkpoint = ran(&snap, &["undo", "restore"]);
    checkpoint.captures.push(keyjutsu_core::recovery::StepCapture {
        step: "restore".into(),
        step_hash: snap.step_hashes()["restore"].clone(),
        captured_at: AT.into(),
        items: vec![keyjutsu_core::recovery::Captured::RegistryValue {
            target: r"HKLM:\SOFTWARE\KeyJutsu-Tests\Value".into(),
            existed: false,
            value_kind: None,
            value_json: None,
        }],
    });

    let items = keyjutsu_core::recovery::plan_recovery_as(&snap, &checkpoint, &[], false).unwrap();
    match &items[0] {
        RecoveryItem::Cannot { step, reason } => {
            assert_eq!(step, "restore");
            assert!(reason.contains("does not trust"), "{reason}");
        }
        other => panic!("restoring captures unelevated: {other:?}"),
    }
    assert!(matches!(&items[1], RecoveryItem::Commands { step, .. } if step == "undo"));
    // Elevated, KeyJutsu restores the captures itself.
    let elevated = keyjutsu_core::recovery::plan_recovery_as(&snap, &checkpoint, &[], true).unwrap();
    assert!(matches!(&elevated[0], RecoveryItem::Restore { .. }), "{elevated:?}");
    assert!(keyjutsu_core::recovery::needs_broker(&snap, &items));

    // With the broker: it is asked, by step and hash, and nothing else runs.
    let broker = RecordingBroker::default();
    let results = recover(
        None,
        Some(&broker),
        &snap,
        &checkpoint,
        &dir,
        &items[1..],
        &PerformanceConfig::default(),
        &|_| {},
    );
    assert!(results[0].recovered, "{results:?}");
    assert_eq!(*broker.0.lock().unwrap(), [("undo".to_owned(), snap.step_hashes()["undo"].clone())]);

    // Without it: refused, and said why; no terminal was needed to find out.
    let results =
        recover(None, None, &snap, &checkpoint, &dir, &items[1..], &PerformanceConfig::default(), &|_| {});
    assert!(!results[0].recovered);
    assert!(results[0].checks[0].detail.contains("no elevation broker"), "{results:?}");
}
