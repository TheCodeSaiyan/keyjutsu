//! A snapshot or checkpoint is trusted because this account recorded it in
//! the encrypted store, not because its own hashes agree.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::path::{Path, PathBuf};

use keyjutsu_core::approvals::{check_approval, load_checkpoint, record_approval, save_checkpoint};
use keyjutsu_core::execute::{Checkpoint, StepRun};
use keyjutsu_core::plan::{ApprovalBook, ApprovedSnapshot, parse_plan, seal};
use keyjutsu_core::store::Store;
use keyjutsu_core::validation::{Options, validate};
use serde_json::{Value, json};

const AT: &str = "2026-09-25T04:00:00Z";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("approvals").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn plan(command: &str) -> Value {
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [{"id": "a", "title": "a", "objective": "Test.", "kind": "command",
                   "shell": {"kind": "pwsh"}, "commands": [{"text": command}]}]
    })
}

/// Sealed exactly as `keyjutsu plan approve` does, which is also all a forger
/// needs: the library recomputes every hash for whatever plan it is given.
fn sealed(v: &Value) -> ApprovedSnapshot {
    let draft = parse_plan(&v.to_string()).unwrap();
    let report = validate(&draft, Options { dry_run: false, ..Options::default() });
    let validated =
        keyjutsu_core::plan::ValidPlan::revalidate(report.record_in(draft.plan(), AT), false).unwrap();
    let mut book = ApprovalBook::new();
    assert!(book.approve_all_except_critical(&validated, AT).is_empty());
    seal(&validated, &book, None, AT).unwrap()
}

#[test]
fn a_snapshot_runs_only_if_this_account_approved_it() {
    let dir = scratch("approval");
    let store = Store::open(&dir.join("store")).unwrap();
    let approved = sealed(&plan("Get-Date"));
    assert!(check_approval(&store, &approved).is_err(), "nothing recorded yet");
    record_approval(&store, &approved).unwrap();
    check_approval(&store, &approved).unwrap();

    // The file edited and every hash recomputed: consistent, and refused.
    let forged = sealed(&plan("Get-Process"));
    let refused = check_approval(&store, &forged).unwrap_err();
    assert!(refused.contains("not approved by this Windows account"), "{refused}");
    // The same snapshot text, read back from disk, is still the approved one.
    check_approval(&store, &ApprovedSnapshot::from_json(&approved.to_json()).unwrap()).unwrap();

    // Another store, with its own key, knows nothing of this approval.
    let other = Store::open(&dir.join("other")).unwrap();
    assert!(check_approval(&other, &approved).is_err());
}

#[test]
fn an_approval_record_moved_to_another_snapshot_does_not_vouch_for_it() {
    let dir = scratch("moved");
    let root = dir.join("store");
    let store = Store::open(&root).unwrap();
    let approved = sealed(&plan("Get-Date"));
    let forged = sealed(&plan("Get-Process"));
    record_approval(&store, &approved).unwrap();
    let id = |s: &ApprovedSnapshot| s.snapshot_hash().trim_start_matches("sha256:").to_ascii_lowercase();
    std::fs::copy(
        root.join("approval").join(format!("{}.kje", id(&approved))),
        root.join("approval").join(format!("{}.kje", id(&forged))),
    )
    .unwrap();
    let refused = check_approval(&store, &forged).unwrap_err();
    assert!(refused.contains("altered") || refused.contains("not approved"), "{refused}");
}

fn checkpoint_with(snapshot_hash: &str, steps: &[&str]) -> Checkpoint {
    let mut c = Checkpoint::new(snapshot_hash);
    for s in steps {
        c.runs.push(StepRun {
            step: (*s).to_owned(),
            step_hash: format!("hash-of-{s}"),
            succeeded: true,
            exit_code: Some(0),
            started_at: AT.into(),
            finished_at: AT.into(),
            checks: Vec::new(),
        });
    }
    c
}

#[test]
fn an_edited_checkpoint_is_refused() {
    let dir = scratch("checkpoint");
    let store = Store::open(&dir.join("store")).unwrap();
    let path = dir.join("run.checkpoint.json");
    save_checkpoint(&store, &checkpoint_with("s", &["a"]), &path).unwrap();
    assert_eq!(load_checkpoint(&store, &path).unwrap().runs.len(), 1);

    // Marking a step as done so a resume skips it.
    let mut edited = Checkpoint::load(&path).unwrap();
    edited.runs.push(checkpoint_with("s", &["b"]).runs.remove(0));
    edited.save(&path).unwrap();
    let refused = load_checkpoint(&store, &path).unwrap_err();
    assert!(refused.contains("changed since KeyJutsu wrote it"), "{refused}");

    // A checkpoint written somewhere KeyJutsu never wrote one.
    let elsewhere = dir.join("elsewhere.checkpoint.json");
    std::fs::copy(&path, &elsewhere).unwrap();
    assert!(load_checkpoint(&store, &elsewhere).unwrap_err().contains("no record"));
}

#[test]
fn a_crash_between_recording_and_writing_leaves_a_checkpoint_that_loads() {
    let dir = scratch("crash");
    let store = Store::open(&dir.join("store")).unwrap();
    let path = dir.join("run.checkpoint.json");
    let before = checkpoint_with("s", &["a"]);
    save_checkpoint(&store, &before, &path).unwrap();
    save_checkpoint(&store, &checkpoint_with("s", &["a", "b"]), &path).unwrap();
    // The record of the second save made it; the file did not.
    before.save(&path).unwrap();
    assert_eq!(load_checkpoint(&store, &path).unwrap(), before);
}

/// Failure injection: several processes opening a new store at once must end
/// up with one key. If each made its own and the last one written won,
/// records made under the others could no longer be read.
#[test]
fn a_store_opened_by_many_at_once_keeps_one_key() {
    for round in 0..5 {
        let root = scratch(&format!("race-{round}"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (root, barrier) = (root.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    let store = Store::open(&root).unwrap();
                    store.put("race", &format!("r{i}"), &i).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let store = Store::open(&root).unwrap();
        for i in 0..8 {
            assert_eq!(store.get::<i32>("race", &format!("r{i}")).unwrap(), Some(i), "round {round}");
        }
    }
}

/// A run folder as the desktop app writes it: the snapshot, and beside it a
/// checkpoint that stopped after phase `one` for a Windows restart.
fn waiting_run(store: &Store, dir: &Path, snapshot: &ApprovedSnapshot, at: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("snapshot.json"), snapshot.to_json()).unwrap();
    let mut cp = Checkpoint::new(snapshot.snapshot_hash());
    cp.boundary = Some(keyjutsu_core::boundary::BoundaryWait {
        after_phase: "one".into(),
        kind: keyjutsu_core::plan::model::Boundary::WindowsRestart,
        identity: None,
        recorded_at: at.into(),
    });
    let path = dir.join("snapshot.checkpoint.json");
    save_checkpoint(store, &cp, &path).unwrap();
    path
}

#[test]
fn after_a_restart_only_a_run_this_account_approved_and_stopped_is_offered() {
    use keyjutsu_core::boundary::find_waiting;
    let dir = scratch("waiting");
    let store = Store::open(&dir.join("store")).unwrap();

    let older = sealed(&plan("Get-Date"));
    record_approval(&store, &older).unwrap();
    let a = waiting_run(&store, &dir.join("a"), &older, "2026-09-25T04:00:00Z");
    let newer = sealed(&plan("Get-Location"));
    record_approval(&store, &newer).unwrap();
    let b = waiting_run(&store, &dir.join("b"), &newer, "2026-09-25T05:00:00Z");
    let found = find_waiting(&store, [a.clone(), b.clone()]).unwrap();
    assert_eq!(found.checkpoint_path, b, "the most recent wait is the one offered");
    assert_eq!(found.snapshot.snapshot_hash(), newer.snapshot_hash());

    // A snapshot this account never approved, with a checkpoint that is
    // KeyJutsu's own: never offered.
    let unapproved = sealed(&plan("Get-Process"));
    let c = waiting_run(&store, &dir.join("c"), &unapproved, "2026-09-25T06:00:00Z");
    assert_eq!(find_waiting(&store, [c.clone()]).map(|w| w.checkpoint_path), None);

    // A checkpoint edited after KeyJutsu wrote it: never offered.
    let text = std::fs::read_to_string(&b).unwrap().replace("05:00:00", "07:00:00");
    std::fs::write(&b, text).unwrap();
    let found = find_waiting(&store, [a.clone(), b, c]).unwrap();
    assert_eq!(found.checkpoint_path, a);

    // A run that crossed its boundary no longer waits.
    let mut crossed = load_checkpoint(&store, &a).unwrap();
    crossed.boundary = None;
    save_checkpoint(&store, &crossed, &a).unwrap();
    assert!(find_waiting(&store, [a, dir.join("missing").join("snapshot.checkpoint.json")]).is_none());
}
