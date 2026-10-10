//! `keyjutsu profile` and the `--profile` option, through the real binary.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![allow(clippy::disallowed_methods)] // Tests start programs directly; no window matters here.

use std::path::Path;
use std::process::Command;

fn keyjutsu(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_keyjutsu"))
        .args(args)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn the_six_profiles_are_listed_with_what_each_extends() {
    let (code, out, _) = keyjutsu(&["profile", "list"]);
    assert_eq!(code, 0);
    for line in
        ["developer          Developer\n", "audit-only         Audit only, extends security-response\n"]
    {
        assert!(out.contains(line), "{out}");
    }
    assert_eq!(out.lines().count(), 6, "{out}");
}

#[test]
fn show_names_where_each_rule_comes_from() {
    let (code, out, _) = keyjutsu(&["profile", "show", "security-response"]);
    assert_eq!(code, 0);
    assert!(out.contains("Chain: security-response (built into KeyJutsu) → it-operations"), "{out}");
    assert!(out.contains("refused: Format-Volume"), "{out}");
    assert!(out.contains("from helpdesk"), "{out}");
    assert!(out.contains("This plan is for Security response work"), "{out}");
}

#[test]
fn an_unknown_profile_is_refused_before_anything_else() {
    let (code, _, err) =
        keyjutsu(&["plan", "validate", "docs/examples/check-a-service.json", "--profile", "lenient"]);
    assert_eq!(code, 2);
    assert!(err.contains("there is no profile called \"lenient\""), "{err}");
}

#[cfg(windows)]
#[test]
fn validate_applies_the_profile_it_is_given() {
    let plan = "docs/examples/clear-build-cache.json";
    let (code, out, _) = keyjutsu(&["plan", "validate", plan, "--no-dry-run", "--profile", "helpdesk"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("✕ work profile: Helpdesk needs a recovery"), "{out}");

    // The same plan without a profile has no such finding.
    let (_, out, _) = keyjutsu(&["plan", "validate", plan, "--no-dry-run"]);
    assert!(!out.contains("work profile"), "{out}");
}
