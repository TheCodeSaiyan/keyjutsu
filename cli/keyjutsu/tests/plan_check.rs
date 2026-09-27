//! `keyjutsu plan check`, run as the real binary against the schema fixtures.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![allow(clippy::disallowed_methods)] // Tests start programs directly; no window matters here.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture(kind: &str, name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/plan/v1/examples").join(kind).join(name)
}

fn check(args: &[&str], file: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keyjutsu"))
        .args(["plan", "check"])
        .args(args)
        .arg(file)
        .output()
        .unwrap()
}

#[test]
fn a_valid_plan_prints_its_steps_in_execution_order() {
    let out = check(&[], &fixture("valid", "docker-backend-branch.json"));
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let positions: Vec<usize> = ["detect-backend", "wsl-path", "hyperv-path", "verify"]
        .iter()
        .map(|s| text.find(s).unwrap())
        .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{text}");
    assert!(text.contains("(conditional)"));
}

#[test]
fn a_structural_problem_fails_with_the_problem_named() {
    let out = check(&[], &fixture("structure-invalid", "cycle.json"));
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("the steps form a cycle"), "{err}");
}

#[test]
fn proposals_and_stored_plans_are_held_to_different_rules() {
    let file = fixture("invalid", "agent-claims-readiness.json");
    let out = check(&[], &file);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8(out.stderr).unwrap().contains("only by KeyJutsu"));
    assert!(check(&["--stored"], &file).status.success());
}

#[test]
fn a_missing_file_is_a_usage_error() {
    let out = check(&[], Path::new("no-such-plan.json"));
    assert_eq!(out.status.code(), Some(2));
}
