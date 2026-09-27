//! `keyjutsu git diff`: KeyJutsu's changes alone.

use std::path::Path;
use std::process::ExitCode;

use keyjutsu_core::git::{self, RepoState};

use crate::run_cli::git_folder;

pub fn diff(snapshot: &Path) -> ExitCode {
    let dir = git_folder(snapshot);
    let baseline: Vec<RepoState> = match std::fs::read_to_string(dir.join("baseline.json"))
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
    {
        Ok(b) => b,
        Err(e) => {
            eprintln!("keyjutsu: no Git record for {}: {e}", snapshot.display());
            return ExitCode::FAILURE;
        }
    };
    // The copies it compares against are encrypted with the store's key.
    let store = keyjutsu_core::store::Store::open(&keyjutsu_core::store::default_root()).ok();
    let mut failed = false;
    for (i, before) in baseline.iter().enumerate() {
        match git::report(before, &dir.join(i.to_string()), store.as_ref()) {
            Ok(r) => {
                println!("# {}", r.root);
                if r.keyjutsu.is_empty() {
                    println!("# KeyJutsu changed nothing here.");
                }
                for k in &r.keyjutsu {
                    if k.was_already_changed {
                        println!(
                            "# {}: you had changed it before the run; this is KeyJutsu's part only.",
                            k.path
                        );
                    }
                    print!("{}", k.diff);
                }
                if !r.untouched.is_empty() {
                    println!("# Your own changes, not shown and untouched: {}", r.untouched.join(", "));
                }
            }
            Err(e) => {
                eprintln!("keyjutsu: {}: {e}", before.root);
                failed = true;
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
