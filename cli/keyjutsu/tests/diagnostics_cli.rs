//! `keyjutsu diagnostics`: the bundle is shown before it is saved, is saved
//! only where asked, and does not name this machine or whoever is using it.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![allow(clippy::disallowed_methods)] // Tests start programs directly; no window matters here.

use std::path::PathBuf;
use std::process::{Command, Output};

fn dir(name: &str) -> PathBuf {
    let d = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn keyjutsu(store: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keyjutsu")).args(args).env("KEYJUTSU_STORE", store).output().unwrap()
}

#[test]
fn the_preview_names_neither_the_user_nor_the_machine() {
    let d = dir("diagnostics-preview");
    let out = keyjutsu(&d.join("store"), &["diagnostics", "preview"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("KeyJutsu diagnostic bundle"), "{text}");
    assert!(text.contains("Checks") && text.contains("Terminal probes") && text.contains("Agents"), "{text}");
    let words: Vec<String> =
        text.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '.').map(str::to_lowercase).collect();
    for var in ["USERNAME", "COMPUTERNAME"] {
        if let Ok(value) = std::env::var(var) {
            assert!(!words.contains(&value.to_lowercase()), "{var} is in the bundle:\n{text}");
        }
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        assert!(
            !text.to_lowercase().contains(&profile.to_lowercase()),
            "the profile folder is in the bundle"
        );
    }
}

#[test]
fn save_writes_the_bundle_and_will_not_replace_a_file_unasked() {
    let d = dir("diagnostics-save");
    let file = d.join("bundle.txt");
    let path = file.to_str().unwrap();
    let out = keyjutsu(&d.join("store"), &["diagnostics", "save", path]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Nothing has been sent"));
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.starts_with("KeyJutsu diagnostic bundle"), "{saved}");

    std::fs::write(&file, "someone else's notes").unwrap();
    let out = keyjutsu(&d.join("store"), &["diagnostics", "save", path]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--force"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "someone else's notes");

    let out = keyjutsu(&d.join("store"), &["diagnostics", "save", path, "--force"]);
    assert!(out.status.success());
    assert!(std::fs::read_to_string(&file).unwrap().starts_with("KeyJutsu diagnostic bundle"));
}
