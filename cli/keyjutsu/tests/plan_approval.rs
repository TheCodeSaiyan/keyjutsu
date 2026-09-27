//! `keyjutsu plan validate | approve | verify | diff`, run as the real binary.
//!
//! The plans are built at run time from what every Windows machine has, so a
//! CI runner without Docker or WSL, or without Administrator rights, gets the
//! same answers as a developer's machine.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![allow(clippy::disallowed_methods)] // Tests start programs directly; no window matters here.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

/// A scratch directory of this test's own, under the build directory.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli").join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn keyjutsu(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keyjutsu"))
        .args(args)
        .env("KEYJUTSU_STORE", std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-store"))
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn plan(steps: Value) -> Value {
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": steps
    })
}

fn step(id: &str, title: &str, command: &str) -> Value {
    json!({"id": id, "title": title, "objective": "Test.", "kind": "command", "shell": {"kind": "pwsh"}, "commands": [{"text": command}]})
}

fn write(dir: &Path, name: &str, v: &Value) -> String {
    let p = dir.join(name);
    std::fs::write(&p, serde_json::to_string_pretty(v).unwrap()).unwrap();
    p.to_str().unwrap().to_owned()
}

fn read_only_plan() -> Value {
    plan(json!([
        step("date", "Show the date", "Get-Date"),
        step("svc", "Show WMI", "Get-Service -Name Winmgmt")
    ]))
}

#[test]
fn a_critical_step_is_only_sealed_with_its_typed_phrase() {
    let dir = scratch("critical");
    let doomed = dir.join("doomed");
    std::fs::create_dir_all(&doomed).unwrap();
    std::fs::write(doomed.join("x.txt"), "x").unwrap();
    let command = format!(
        "Remove-Item -Recurse -Force -LiteralPath {}",
        doomed.display().to_string().replace('\\', "/")
    );
    // The agent says "low"; KeyJutsu's own rules say critical.
    let mut wipe = step("wipe", "Remove the scratch tree", &command);
    wipe["proposed_risk"] = json!({"level": "normal", "rationale": "Tidy up."});
    let file = write(&dir, "plan.json", &plan(json!([step("date", "Show the date", "Get-Date"), wipe])));
    let out = dir.join("snap.json");
    let out = out.to_str().unwrap();

    // Understated risk puts the step in review, so nothing is sealed.
    let refused = keyjutsu(&["plan", "approve", &file, "--out", out]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(text(&refused).contains("REVIEW"), "{}", text(&refused));
    assert!(doomed.join("x.txt").exists(), "validation must not have deleted anything");

    // Once the plan states the risk honestly, the typed phrase is still needed.
    let mut honest = plan(json!([
        step("date", "Show the date", "Get-Date"),
        step("wipe", "Remove the scratch tree", &command)
    ]));
    honest["steps"][1]["proposed_risk"] =
        json!({"level": "critical", "rationale": "Deletes the scratch tree."});
    let file = write(&dir, "honest.json", &honest);
    let needs_phrase = keyjutsu(&["plan", "approve", &file, "--out", out]);
    assert_eq!(needs_phrase.status.code(), Some(1));
    let t = text(&needs_phrase);
    assert!(t.contains("CRITICAL ACTION") && t.contains("REMOVE THE SCRATCH TREE"), "{t}");
    assert!(t.contains("What if:"), "the dry run's expanded target is shown: {t}");
    assert!(!Path::new(out).exists(), "nothing is written when sealing is refused");

    let wrong = keyjutsu(&["plan", "approve", &file, "--out", out, "--confirm", "wipe=yes"]);
    assert_eq!(wrong.status.code(), Some(1));
    let ok = keyjutsu(&["plan", "approve", &file, "--out", out, "--confirm", "wipe=REMOVE THE SCRATCH TREE"]);
    assert!(ok.status.success(), "{}", text(&ok));
    assert!(keyjutsu(&["plan", "verify", out]).status.success());
    assert!(doomed.join("x.txt").exists(), "approving runs nothing");
}

#[test]
fn a_plan_that_cannot_run_here_is_not_sealed() {
    let dir = scratch("blocked");
    let file =
        write(&dir, "plan.json", &plan(json!([step("typo", "Mistyped", "Get-Service -Nmae Winmgmt")])));
    let v = keyjutsu(&["plan", "validate", &file]);
    assert_eq!(v.status.code(), Some(1));
    assert!(text(&v).contains("INVALID") && text(&v).contains("-Nmae"), "{}", text(&v));
    let out = dir.join("snap.json");
    let a = keyjutsu(&["plan", "approve", &file, "--out", out.to_str().unwrap()]);
    assert_eq!(a.status.code(), Some(1));
    assert!(!out.exists());
}

#[test]
fn an_edited_snapshot_fails_verification() {
    let dir = scratch("tamper");
    let file = write(&dir, "plan.json", &read_only_plan());
    let snap = dir.join("snap.json");
    let out = snap.to_str().unwrap();
    let approved = keyjutsu(&["plan", "approve", &file, "--out", out]);
    assert!(approved.status.success(), "{}", text(&approved));

    let original = std::fs::read_to_string(&snap).unwrap();
    std::fs::write(&snap, original.replace("Get-Date", "Stop-Computer")).unwrap();
    let verify = keyjutsu(&["plan", "verify", out]);
    assert_eq!(verify.status.code(), Some(1));
    assert!(text(&verify).contains("altered"), "{}", text(&verify));
}

/// A snapshot with consistent hashes is still refused unless
/// this account approved it here. Another store stands in for another
/// account or machine: it has its own key and none of these approvals.
#[test]
fn run_refuses_a_snapshot_this_account_did_not_approve() {
    let dir = scratch("elsewhere");
    let file = write(&dir, "plan.json", &read_only_plan());
    let snap = dir.join("snap.json");
    let approved = keyjutsu(&["plan", "approve", &file, "--out", snap.to_str().unwrap()]);
    assert!(approved.status.success(), "{}", text(&approved));
    for command in [&["run", snap.to_str().unwrap()][..], &["recover", snap.to_str().unwrap()][..]] {
        let refused = Command::new(env!("CARGO_BIN_EXE_keyjutsu"))
            .args(command)
            .env("KEYJUTSU_STORE", dir.join("another-account"))
            .output()
            .unwrap();
        assert_eq!(refused.status.code(), Some(1), "{command:?}: {}", text(&refused));
        assert!(text(&refused).contains("not approved by this Windows account"), "{}", text(&refused));
    }
}

/// While one run changes the machine, another is refused before it starts
/// anything, broker and UAC prompt included.
#[test]
fn a_second_run_is_refused_while_one_changes_the_machine() {
    let dir = scratch("one-at-a-time");
    let file = write(&dir, "plan.json", &read_only_plan());
    let snap = dir.join("snap.json");
    let approved = keyjutsu(&["plan", "approve", &file, "--out", snap.to_str().unwrap()]);
    assert!(approved.status.success(), "{}", text(&approved));
    let name = format!(r"Local\keyjutsu-cli-test-held-{}", std::process::id());
    let running = keyjutsu_core::runlock::RunLock::take_named(&name).unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_keyjutsu"))
        .args(["run", snap.to_str().unwrap()])
        .env("KEYJUTSU_STORE", std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-store"))
        .env("KEYJUTSU_RUN_LOCK", &name)
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1), "{}", text(&refused));
    assert!(text(&refused).contains("another KeyJutsu run is changing this machine"), "{}", text(&refused));
    assert!(!dir.join("snap.checkpoint.json").exists(), "nothing started");
    drop(running);
}

#[test]
fn an_existing_snapshot_is_not_overwritten_without_force() {
    let dir = scratch("force");
    let file = write(&dir, "plan.json", &read_only_plan());
    let snap = dir.join("snap.json");
    std::fs::write(&snap, "keep me").unwrap();
    let out = keyjutsu(&["plan", "approve", &file, "--out", snap.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(std::fs::read_to_string(&snap).unwrap(), "keep me");
    let forced = keyjutsu(&["plan", "approve", &file, "--out", snap.to_str().unwrap(), "--force"]);
    assert!(forced.status.success(), "{}", text(&forced));
}

#[test]
fn diff_names_the_steps_that_need_revalidation() {
    let dir = scratch("diff");
    let old = write(&dir, "old.json", &read_only_plan());
    let mut changed = read_only_plan();
    changed["steps"][0]["commands"][0]["text"] = "Get-Date -Format o".into();
    let new = write(&dir, "new.json", &changed);
    let out = keyjutsu(&["plan", "diff", &old, &new]);
    assert!(out.status.success());
    let t = text(&out);
    assert!(t.contains("changed  date: commands"), "{t}");
    assert!(t.contains("2 steps require revalidation"), "date and svc after it: {t}");
}

/// `plan revise --session` reads the failure from the encrypted history, and
/// refuses a session that failed at another step or did not fail at all.
#[test]
fn a_revision_takes_its_failure_from_the_recorded_session() {
    use keyjutsu_core::execute::Outcome;
    use keyjutsu_core::history::{SessionRecord, save};
    let dir = scratch("revise-session");
    let file = write(&dir, "plan.json", &read_only_plan());
    let store =
        keyjutsu_core::store::Store::open(&Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-store")).unwrap();
    let record = |id: &str, outcome: Outcome| SessionRecord {
        id: id.into(),
        started_at: "2026-09-25T00:00:00Z".into(),
        finished_at: "2026-09-25T00:01:00Z".into(),
        task: "t".into(),
        agent: serde_json::from_value(json!({"name": "codex", "version": "1"})).unwrap(),
        snapshot: String::new(),
        checkpoint: None,
        outcome,
        git: Vec::new(),
    };
    let failed = |step: &str| Outcome::Failed {
        step: step.into(),
        expected: "exit code 0".into(),
        actual: "a command exited with 1".into(),
        output: "it broke".into(),
    };
    save(&store, &record("20260925-000100-aaaa", failed("other"))).unwrap();
    save(&store, &record("20260925-000100-bbbb", Outcome::Complete)).unwrap();
    let revise = |session: &str| {
        keyjutsu(&[
            "plan",
            "revise",
            &file,
            "--step",
            "look",
            "--guidance",
            "g",
            "--session",
            session,
            "--agent",
            "codex",
            "--out",
            "unused.json",
        ])
    };
    let wrong_step = revise("20260925-000100-aaaa");
    assert_eq!(wrong_step.status.code(), Some(1));
    assert!(
        text(&wrong_step).contains("it was step `other` that failed, not `look`"),
        "{}",
        text(&wrong_step)
    );
    let not_failed = revise("20260925-000100-bbbb");
    assert!(text(&not_failed).contains("did not end in a failed step"), "{}", text(&not_failed));
}
