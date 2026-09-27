//! Capture and recovery touch exactly the file a plan named, never what a
//! link leads to. For an Administrator step the broker does both as
//! Administrator, so a junction planted by anything running as the user,
//! between the capture and the recovery, would otherwise let it write or
//! delete as Administrator wherever the junction points. These tests plant
//! real junctions and hard links (neither needs Administrator to make) and
//! check that nothing on the far side is read, written, made or removed.

#![cfg(windows)]
#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::path::{Path, PathBuf};

use keyjutsu_core::plan::model::Step;
use keyjutsu_core::plan::parse_plan;
use keyjutsu_core::recovery::{Backups, capture_step, restore_capture};
use serde_json::json;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("links").join(name);
    // A junction left by an earlier run is removed as a link, not followed.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn junction(link: &Path, target: &Path) {
    let out = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(out.status.success(), "mklink: {}", String::from_utf8_lossy(&out.stdout));
}

/// A step that captures `target` before it runs.
fn capturing(target: &Path) -> Step {
    let plan = json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": [{
            "id": "edit", "title": "Edit", "objective": "Test.", "kind": "command",
            "shell": {"kind": "pwsh"}, "commands": [{"text": "Get-Date"}],
            "reversibility": {"level": "full"},
            "recovery": {"strategy": "restore_captured_state",
                         "capture": [{"kind": "file", "target": target.display().to_string()}]}
        }]
    });
    parse_plan(&plan.to_string()).unwrap().plan().step("edit").unwrap().clone()
}

/// `work\app\config.txt` as the step found it, and a protected folder the
/// user must not be able to reach through KeyJutsu.
fn setting(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = scratch(name);
    let app = root.join("work").join("app");
    let protected = root.join("protected");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::create_dir_all(&protected).unwrap();
    (root, app, protected)
}

#[test]
fn a_restore_never_writes_through_a_junction_planted_after_the_capture() {
    let (root, app, protected) = setting("write");
    let target = app.join("config.txt");
    std::fs::write(&target, "chosen by whoever could write here").unwrap();
    let step = capturing(&target);
    let captured = capture_step(&step, "h", Backups::plain(&root.join("backups")), "now".into()).unwrap();

    // The folder on the way is swapped for a junction to the protected one:
    // once where the file is there, once where it is not.
    std::fs::rename(&app, root.join("work").join("app-moved")).unwrap();
    junction(&app, &protected);
    std::fs::write(protected.join("config.txt"), "precious").unwrap();
    let checks = restore_capture(&captured, Backups::plain(&root.join("backups")));
    assert!(checks.iter().all(|c| c.passed == Some(false)), "{checks:?}");
    assert!(checks[0].detail.contains("link or junction"), "{checks:?}");
    assert_eq!(std::fs::read_to_string(protected.join("config.txt")).unwrap(), "precious");

    std::fs::remove_file(protected.join("config.txt")).unwrap();
    let checks = restore_capture(&captured, Backups::plain(&root.join("backups")));
    assert!(checks.iter().all(|c| c.passed == Some(false)), "{checks:?}");
    assert!(!protected.join("config.txt").exists(), "nothing is made on the far side");
}

#[test]
fn a_restore_never_deletes_through_a_junction() {
    let (root, app, protected) = setting("delete");
    let target = app.join("new.txt");
    let step = capturing(&target);
    // It did not exist before the step, so recovery would remove it.
    let captured = capture_step(&step, "h", Backups::plain(&root.join("backups")), "now".into()).unwrap();

    std::fs::remove_dir(&app).unwrap();
    junction(&app, &protected);
    std::fs::write(protected.join("new.txt"), "precious").unwrap();
    let checks = restore_capture(&captured, Backups::plain(&root.join("backups")));
    assert!(checks.iter().all(|c| c.passed == Some(false)), "{checks:?}");
    assert_eq!(std::fs::read_to_string(protected.join("new.txt")).unwrap(), "precious");
}

#[test]
fn a_capture_never_reads_through_a_junction() {
    let (root, app, protected) = setting("read");
    std::fs::remove_dir(&app).unwrap();
    junction(&app, &protected);
    std::fs::write(protected.join("config.txt"), "secret").unwrap();
    let err = capture_step(
        &capturing(&app.join("config.txt")),
        "h",
        Backups::plain(&root.join("backups")),
        "now".into(),
    )
    .unwrap_err();
    assert!(err.contains("link or junction"), "{err}");
    assert!(!root.join("backups").join("edit-0.bak").exists(), "no copy was kept");
}

#[test]
fn a_file_with_a_second_name_is_neither_captured_nor_restored() {
    let (root, app, protected) = setting("hardlink");
    std::fs::write(protected.join("config.txt"), "precious").unwrap();
    let target = app.join("config.txt");
    std::fs::hard_link(protected.join("config.txt"), &target).unwrap();
    let err = capture_step(&capturing(&target), "h", Backups::plain(&root.join("backups")), "now".into())
        .unwrap_err();
    assert!(err.contains("hard link"), "{err}");

    // Captured as a plain file, then made a second name of the protected one.
    std::fs::remove_file(&target).unwrap();
    std::fs::write(&target, "mine").unwrap();
    let captured =
        capture_step(&capturing(&target), "h", Backups::plain(&root.join("backups")), "now".into()).unwrap();
    std::fs::remove_file(&target).unwrap();
    std::fs::hard_link(protected.join("config.txt"), &target).unwrap();
    let checks = restore_capture(&captured, Backups::plain(&root.join("backups")));
    assert!(checks.iter().all(|c| c.passed == Some(false)), "{checks:?}");
    assert_eq!(std::fs::read_to_string(protected.join("config.txt")).unwrap(), "precious");
}

/// On a CI runner the temporary folder is `C:\Users\RUNNER~1\…`: a short
/// name is the same folder, not a link, and is not refused.
#[test]
fn a_file_named_by_its_short_name_is_still_the_same_file() {
    use std::os::windows::process::CommandExt;
    let long = std::env::temp_dir().join(format!("keyjutsu exact file long name {}", std::process::id()));
    std::fs::create_dir_all(&long).unwrap();
    let out = std::process::Command::new("cmd")
        .raw_arg(format!("/d /c for %I in (\"{}\") do @echo %~sI", long.display()))
        .output()
        .unwrap();
    let short = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    if short == long || !short.exists() {
        // 8.3 names are switched off on this volume; nothing to compare.
        let _ = std::fs::remove_dir_all(&long);
        return;
    }
    let target = short.join("config.txt");
    std::fs::write(&target, "before").unwrap();
    let captured =
        capture_step(&capturing(&target), "h", Backups::plain(&long.join("backups")), "now".into()).unwrap();
    std::fs::write(&target, "after").unwrap();
    let checks = restore_capture(&captured, Backups::plain(&long.join("backups")));
    assert!(checks.iter().all(|c| c.passed == Some(true)), "{checks:?}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "before");
    let _ = std::fs::remove_dir_all(&long);
}

/// A recovery backup is a copy of the operator's file, secrets and all: kept
/// encrypted with the store's key, and restored through it (ADR 0018). A
/// plain backup, from the broker or an earlier run, is still restored.
#[test]
fn a_recovery_backup_is_kept_encrypted_and_still_restores() {
    let (root, app, _) = setting("encrypted");
    let target = app.join("settings.json");
    let secret = r#"{"password": "do-not-back-me-up-in-plain-text"}"#;
    std::fs::write(&target, secret).unwrap();
    let store = keyjutsu_core::store::Store::open(&root.join("store")).unwrap();
    let dir = root.join("backups");
    let captured =
        capture_step(&capturing(&target), "h", Backups::sealed(&dir, &store), "now".into()).unwrap();
    let kept = std::fs::read(dir.join("edit-0.bak")).unwrap();
    assert!(!String::from_utf8_lossy(&kept).contains("do-not-back-me-up"), "the backup is plain");

    std::fs::write(&target, "{}").unwrap();
    let without = restore_capture(&captured, Backups::plain(&dir));
    assert!(without.iter().all(|c| c.passed == Some(false)), "not without the store: {without:?}");
    let checks = restore_capture(&captured, Backups::sealed(&dir, &store));
    assert!(checks.iter().all(|c| c.passed == Some(true)), "{checks:?}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), secret);

    // Another account's or machine's store cannot open it.
    let other = keyjutsu_core::store::Store::open(&root.join("other-store")).unwrap();
    std::fs::write(&target, "{}").unwrap();
    let foreign = restore_capture(&captured, Backups::sealed(&dir, &other));
    assert!(foreign.iter().all(|c| c.passed == Some(false)), "{foreign:?}");

    // A plain backup is still read.
    let plain = capture_step(&capturing(&target), "h", Backups::plain(&dir), "now".into()).unwrap();
    std::fs::write(&target, "changed").unwrap();
    let checks = restore_capture(&plain, Backups::sealed(&dir, &store));
    assert!(checks.iter().all(|c| c.passed == Some(true)), "{checks:?}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}");
}

#[test]
fn a_plain_file_is_captured_and_restored_as_before() {
    let (root, app, _) = setting("plain");
    let target = app.join("config.txt");
    std::fs::write(&target, "before").unwrap();
    let captured =
        capture_step(&capturing(&target), "h", Backups::plain(&root.join("backups")), "now".into()).unwrap();
    std::fs::write(&target, "after").unwrap();
    let checks = restore_capture(&captured, Backups::plain(&root.join("backups")));
    assert!(checks.iter().all(|c| c.passed == Some(true)), "{checks:?}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "before");

    let fresh = app.join("fresh.txt");
    let captured =
        capture_step(&capturing(&fresh), "h", Backups::plain(&root.join("backups")), "now".into()).unwrap();
    std::fs::write(&fresh, "made by the step").unwrap();
    let checks = restore_capture(&captured, Backups::plain(&root.join("backups")));
    assert!(checks.iter().all(|c| c.passed == Some(true)), "{checks:?}");
    assert!(!fresh.exists());
}
