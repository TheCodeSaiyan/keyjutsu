//! Milestone 13: a plan run in a dirty repository, with KeyJutsu's changes
//! told apart from the operator's. Real git, real pwsh, a throwaway repo.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use keyjutsu_core::execute::{Driver, ExecuteOptions, ForwardingSink, Outcome, execute};
use keyjutsu_core::execution::ExecutionMode;
use keyjutsu_core::git::{create_worktree, find_root, record, report, repositories, steps_inside};
use keyjutsu_core::headless::Collector;
use keyjutsu_core::plan::{ApprovalBook, ApprovedSnapshot, parse_plan, seal};
use keyjutsu_core::terminal::{ProfileMode, ShellKind};
use keyjutsu_core::validation::{Options, validate};
use keyjutsu_core::{Session, SessionOptions};
use serde_json::{Value, json};

const AT: &str = "2026-09-25T07:00:00Z";

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("git").join(name);
    if dir.exists() {
        // Worktrees are registered in their repository; forget them first.
        let _ = std::fs::remove_dir_all(&dir);
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=KeyJutsu Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repository with three committed files, then the operator's own work:
/// a.txt edited, c.txt deleted, u.txt new and untracked.
fn dirty_repo(name: &str) -> PathBuf {
    let repo = scratch(name).join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "core.autocrlf", "false"]);
    for f in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(repo.join(f), format!("{f} line 1\n")).unwrap();
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "start"]);
    std::fs::write(repo.join("a.txt"), "a.txt line 1\nuser-a\n").unwrap();
    std::fs::remove_file(repo.join("c.txt")).unwrap();
    std::fs::write(repo.join("u.txt"), "mine\n").unwrap();
    repo
}

fn plan(steps: Value) -> Value {
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": steps
    })
}

fn step(id: &str, command: &str) -> Value {
    json!({"id": id, "title": id, "objective": "Test.", "kind": "command", "shell": {"kind": "pwsh"},
           "commands": [{"text": command}]})
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

fn run_in(dir: &Path, snap: &ApprovedSnapshot) -> Outcome {
    let (tx, events) = channel();
    let sink = Arc::new(ForwardingSink { inner: Arc::new(Collector::new()), events: Mutex::new(tx) });
    let mut o = SessionOptions::new(ShellKind::Pwsh);
    o.profile = ProfileMode::Clean;
    o.intercept_cursor_queries = true;
    o.cwd = Some(dir.to_owned());
    let session = Session::open(o, sink).unwrap();
    assert!(session.wait_ready(Duration::from_secs(30)));
    let options = ExecuteOptions { mode: Some(ExecutionMode::Direct), ..ExecuteOptions::default() };
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
fn keyjutsus_changes_are_told_apart_from_the_operators_in_a_dirty_repository() {
    let repo = dirty_repo("dirty");
    let copies = repo.parent().unwrap().join("copies");
    let snap = approve(&plan(json!([
        step("edit-clean", "Add-Content -LiteralPath b.txt -Value keyjutsu-b"),
        step("touch-yours", "Add-Content -LiteralPath a.txt -Value keyjutsu-a"),
        step("create", "Set-Content -LiteralPath k.txt -Value new"),
    ])));
    let roots = repositories(&repo, snap.plan());
    assert_eq!(roots.len(), 1);
    let before = record(&roots[0], Some(&copies)).unwrap();
    assert_eq!(before.branch.as_deref(), Some("main"));
    assert_eq!(before.dirty.len(), 3, "{:?}", before.dirty);

    assert_eq!(run_in(&repo, &snap), Outcome::Complete);
    let r = report(&before, &copies).unwrap();

    let mine: Vec<&str> = r.untouched.iter().map(String::as_str).collect();
    assert_eq!(mine, ["c.txt", "u.txt"], "the operator's changes KeyJutsu did not touch");
    let ours: Vec<(&str, bool)> =
        r.keyjutsu.iter().map(|k| (k.path.as_str(), k.was_already_changed)).collect();
    assert_eq!(ours, [("a.txt", true), ("b.txt", false), ("k.txt", false)]);

    let diff = |p: &str| r.keyjutsu.iter().find(|k| k.path == p).unwrap().diff.clone();
    let a = diff("a.txt");
    assert!(a.contains("+keyjutsu-a"), "{a}");
    assert!(!a.contains("+user-a"), "the operator's own edit is not KeyJutsu's:\n{a}");
    assert!(a.contains("a/a.txt") && !a.contains("copies"), "labelled by the file, not the copy:\n{a}");
    assert!(diff("b.txt").contains("+keyjutsu-b"));
    assert!(diff("k.txt").contains("+new"));
    assert!(r.head_moved.is_none());
}

#[test]
fn a_commit_made_by_a_step_is_reported_as_keyjutsus() {
    let repo = dirty_repo("commit");
    let copies = repo.parent().unwrap().join("copies");
    let before = record(&repo, Some(&copies)).unwrap();
    // Standing in for an approved commit step.
    std::fs::write(repo.join("b.txt"), "b.txt line 1\ncommitted\n").unwrap();
    git(&repo, &["commit", "-q", "-m", "step", "--", "b.txt"]);
    let r = report(&before, &copies).unwrap();
    assert!(r.head_moved.is_some());
    let b = r.keyjutsu.iter().find(|k| k.path == "b.txt").unwrap();
    assert_eq!(b.status, "committed");
    assert!(b.diff.contains("+committed"), "{}", b.diff);
    assert_eq!(r.untouched, ["a.txt", "c.txt", "u.txt"]);
}

#[test]
fn a_worktree_isolates_a_run_from_the_operators_working_tree() {
    let repo = dirty_repo("worktree");
    let dest = repo.parent().unwrap().join("isolated");
    let made = create_worktree(&repo, "keyjutsu/test-run", &dest).unwrap();
    assert!(made.join("b.txt").exists());
    assert!(!made.join("u.txt").exists(), "uncommitted work stays in the operator's tree");
    assert_eq!(std::fs::read_to_string(made.join("a.txt")).unwrap(), "a.txt line 1\n");
    assert_eq!(
        git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "main",
        "the operator's branch is unchanged"
    );
    assert_eq!(find_root(&made).map(|p| p.file_name().unwrap().to_owned()), Some("isolated".into()));

    // A step naming a folder in the original repository would not be isolated.
    let p: keyjutsu_core::plan::model::Plan = serde_json::from_value(plan(json!([{
        "id": "s", "title": "s", "objective": "o", "kind": "command", "shell": {"kind": "pwsh"},
        "working_directory": repo.display().to_string(), "commands": [{"text": "Get-Date"}]
    }])))
    .unwrap();
    assert_eq!(steps_inside(&repo, &p), ["s"]);
    git(&repo, &["worktree", "remove", "--force", &dest.display().to_string()]);
}

#[test]
fn a_folder_outside_any_repository_has_none() {
    let dir = scratch("plain");
    assert!(find_root(&dir).is_none() || find_root(&dir).is_some_and(|r| !r.starts_with(&dir)));
}
