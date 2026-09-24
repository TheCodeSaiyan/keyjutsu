//! `keyjutsu plan approve | verify | diff`, run as the real binary.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/plan/v1/examples/valid").join(name)
}

/// A scratch directory of this test's own, under the build directory.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn keyjutsu(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keyjutsu")).args(args).output().unwrap()
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn a_critical_step_is_only_sealed_with_its_typed_phrase() {
    let dir = scratch("critical");
    let snap = dir.join("snap.json");
    let (plan, out) = (fixture("critical-reset.json"), snap.to_str().unwrap().to_owned());

    let refused = keyjutsu(&["plan", "approve", plan.to_str().unwrap(), "--out", &out]);
    assert_eq!(refused.status.code(), Some(1));
    let t = text(&refused);
    assert!(t.contains("CRITICAL ACTION") && t.contains("REMOVE LOCAL DOCKER DATA"), "{t}");
    assert!(t.contains("C:/ProgramData/Docker/data") && t.contains("impact"), "shows target and impact: {t}");
    assert!(!snap.exists(), "nothing is written when sealing is refused");

    let wrong =
        keyjutsu(&["plan", "approve", plan.to_str().unwrap(), "--out", &out, "--confirm", "remove-data=yes"]);
    assert_eq!(wrong.status.code(), Some(1));

    let ok = keyjutsu(&[
        "plan",
        "approve",
        plan.to_str().unwrap(),
        "--out",
        &out,
        "--confirm",
        "remove-data=REMOVE LOCAL DOCKER DATA",
    ]);
    assert!(ok.status.success(), "{}", text(&ok));
    assert!(keyjutsu(&["plan", "verify", &out]).status.success());
}

#[test]
fn an_edited_snapshot_fails_verification() {
    let dir = scratch("tamper");
    let snap = dir.join("snap.json");
    let out = snap.to_str().unwrap();
    assert!(
        keyjutsu(&["plan", "approve", fixture("docker-backend-branch.json").to_str().unwrap(), "--out", out])
            .status
            .success()
    );

    let original = std::fs::read_to_string(&snap).unwrap();
    std::fs::write(&snap, original.replace("wsl --shutdown", "wsl --unregister Ubuntu")).unwrap();
    let verify = keyjutsu(&["plan", "verify", out]);
    assert_eq!(verify.status.code(), Some(1));
    assert!(text(&verify).contains("altered"), "{}", text(&verify));
}

#[test]
fn an_existing_snapshot_is_not_overwritten_without_force() {
    let dir = scratch("force");
    let snap = dir.join("snap.json");
    std::fs::write(&snap, "keep me").unwrap();
    let plan = fixture("docker-backend-branch.json");
    let out = keyjutsu(&["plan", "approve", plan.to_str().unwrap(), "--out", snap.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(std::fs::read_to_string(&snap).unwrap(), "keep me");
    let forced =
        keyjutsu(&["plan", "approve", plan.to_str().unwrap(), "--out", snap.to_str().unwrap(), "--force"]);
    assert!(forced.status.success());
}

#[test]
fn diff_names_the_steps_that_need_revalidation() {
    let dir = scratch("diff");
    let old = fixture("docker-backend-branch.json");
    let new = dir.join("new.json");
    let original = std::fs::read_to_string(&old).unwrap();
    std::fs::write(&new, original.replace("wsl --shutdown", "wsl --shutdown --force")).unwrap();
    let out = keyjutsu(&["plan", "diff", old.to_str().unwrap(), new.to_str().unwrap()]);
    assert!(out.status.success());
    let t = text(&out);
    assert!(t.contains("changed  wsl-path: commands"), "{t}");
    assert!(t.contains("2 step(s) require revalidation"), "wsl-path and verify: {t}");
}
