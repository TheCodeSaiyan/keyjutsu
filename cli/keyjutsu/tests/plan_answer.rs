//! `keyjutsu plan answer`, run as the real binary: the agent's questions are
//! listed, and one can be closed without asking the agent anything. Sending
//! an answer goes to an agent, which the workspace tests replay instead.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.
#![allow(clippy::disallowed_methods)] // Tests start programs directly; no window matters here.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn asking() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/plan/v1/examples/valid/questions-for-the-operator.json")
}

fn answer(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_keyjutsu")).args(["plan", "answer"]).args(args).output().unwrap()
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

#[test]
fn the_agents_questions_are_listed_with_their_options() {
    let out = answer(&[asking().to_str().unwrap()]);
    assert!(out.status.success(), "{}", text(&out));
    let said = text(&out);
    assert!(said.contains("which-desktop (step `make-pdf`): Your Desktop is in OneDrive"), "{said}");
    assert!(said.contains("- The local Desktop folder") && said.contains("- or your own answer"), "{said}");
    assert!(said.contains("open-afterwards: Should the PDF be opened"), "{said}");
}

#[test]
fn carrying_on_closes_the_question_and_records_it() {
    let dir = std::env::temp_dir().join(format!("kj-plan-answer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out_file = dir.join("closed.json");
    let out = answer(&[
        asking().to_str().unwrap(),
        "--question",
        "which-desktop",
        "--carry-on",
        "--out",
        out_file.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let plan: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out_file).unwrap()).unwrap();
    let ids: Vec<&str> =
        plan["questions"].as_array().unwrap().iter().map(|q| q["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["open-afterwards"]);
    let events = plan["keyjutsu"]["provenance"].as_array().unwrap();
    assert!(events.iter().any(|e| e["action"] == "answered" && e["step"] == "make-pdf"), "{events:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_answer_needs_an_agent_and_a_known_question() {
    let file = asking();
    let file = file.to_str().unwrap();
    let no_agent = answer(&[file, "--question", "which-desktop", "--answer", "Local", "--out", "x.json"]);
    assert_eq!(no_agent.status.code(), Some(2), "{}", text(&no_agent));
    assert!(text(&no_agent).contains("--agent"));
    let unknown = answer(&[file, "--question", "nowhere", "--carry-on", "--out", "x.json"]);
    assert_eq!(unknown.status.code(), Some(1), "{}", text(&unknown));
    assert!(text(&unknown).contains("no open question `nowhere`"));
    assert!(!Path::new("x.json").exists(), "nothing was written");
}
