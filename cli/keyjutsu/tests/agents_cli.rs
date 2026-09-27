//! `keyjutsu agents` and the agent commands' guards, without ever sending a
//! request: CI runners have no agents, and a developer's machine should not
//! spend their quota on a test.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![allow(clippy::disallowed_methods)] // Tests start programs directly; no window matters here.

use std::process::{Command, Output};

fn keyjutsu(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keyjutsu")).args(args).output().unwrap()
}

#[test]
fn every_supported_agent_is_listed_installed_or_not() {
    let out = keyjutsu(&["agents", "--json"]);
    assert!(out.status.success());
    let list: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let kinds: Vec<&str> = list.as_array().unwrap().iter().map(|a| a["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["codex", "claude_code", "gemini", "github_copilot", "cursor"]);
    for a in list.as_array().unwrap() {
        assert!(a["capabilities"]["read_only"].as_str().unwrap().len() > 10, "{a}");
        // Installed or not, it says where to get it, and it is a page
        // KeyJutsu is willing to open.
        let url = a["install_url"].as_str().unwrap();
        assert!(keyjutsu_core::links::is_known(url), "{url}");
    }
}

#[test]
fn the_live_check_refuses_to_run_without_being_asked_to() {
    let out = keyjutsu(&["agents", "check"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--live"));
}

#[test]
fn an_unknown_agent_is_a_usage_error() {
    let out = keyjutsu(&["plan", "propose", "do something", "--agent", "chatgpt", "--out", "x.json"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn without_send_nothing_leaves_the_machine_and_nothing_is_written() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("agents-cli");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out_file = dir.join("plan.json");
    // Whichever agent is installed here, or none: the request is never sent.
    for agent in ["claude", "codex", "gemini", "copilot"] {
        let out = keyjutsu(&["plan", "propose", "t", "--agent", agent, "--out", out_file.to_str().unwrap()]);
        let text =
            format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            assert!(text.contains("Nothing was sent"), "{text}");
        } else {
            assert!(text.contains("not installed"), "{text}");
        }
        assert!(!out_file.exists(), "{agent}: a plan was written without --send");
    }
}
