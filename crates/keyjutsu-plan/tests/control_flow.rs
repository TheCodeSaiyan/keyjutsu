//! Deciding what runs next, and what a change affects.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::path::Path;

use keyjutsu_plan::model::{Command, FactValue};
use keyjutsu_plan::{KnownFacts, Missing, StepResult, ValidPlan, diff, frontier, parse_plan};
use serde_json::json;

fn fixture(name: &str) -> ValidPlan {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/plan/v1/examples/valid").join(name);
    parse_plan(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn ok() -> StepResult {
    StepResult::Succeeded { exit_code: Some(0) }
}

#[test]
fn a_branch_waits_for_its_fact_then_takes_one_side_and_skips_the_other() {
    let p = fixture("docker-backend-branch.json");
    let mut facts = KnownFacts::default();

    let f = frontier(p.plan(), p.graph(), &facts);
    assert_eq!(f.ready, ["detect-backend"]);

    facts.steps.insert("detect-backend".into(), ok());
    let f = frontier(p.plan(), p.graph(), &facts);
    assert!(f.ready.is_empty(), "never guesses the backend");
    assert_eq!(f.needs, [Missing::Fact("docker.backend".into())]);
    assert!(!f.complete);

    facts.facts.insert("docker.backend".into(), FactValue::Text("wsl2".into()));
    let f = frontier(p.plan(), p.graph(), &facts);
    assert_eq!(f.ready, ["wsl-path"]);
    assert_eq!(f.skipped, ["hyperv-path"]);
    assert!(f.ready.iter().all(|s| s != "verify"), "the join waits for its branch");

    facts.steps.insert("wsl-path".into(), ok());
    let f = frontier(p.plan(), p.graph(), &facts);
    assert_eq!(f.ready, ["verify"], "the branches join again");

    facts.steps.insert("verify".into(), ok());
    let f = frontier(p.plan(), p.graph(), &facts);
    assert!(f.complete && f.ready.is_empty());
}

#[test]
fn the_other_backend_takes_the_other_branch() {
    let p = fixture("docker-backend-branch.json");
    let mut facts = KnownFacts::default();
    facts.steps.insert("detect-backend".into(), ok());
    facts.facts.insert("docker.backend".into(), FactValue::Text("hyperv".into()));
    let f = frontier(p.plan(), p.graph(), &facts);
    assert_eq!(f.ready, ["hyperv-path"]);
    assert_eq!(f.skipped, ["wsl-path"]);
}

#[test]
fn a_failure_halts_the_plan_whatever_the_graph_says() {
    let p = fixture("docker-backend-branch.json");
    let mut facts = KnownFacts::default();
    facts.steps.insert("detect-backend".into(), StepResult::Failed { exit_code: Some(1) });
    let f = frontier(p.plan(), p.graph(), &facts);
    assert_eq!(f.halted_by.as_deref(), Some("detect-backend"));
    assert!(f.ready.is_empty() && !f.complete);
}

#[test]
fn a_step_whose_dependency_was_skipped_is_skipped_too() {
    let mut v: serde_json::Value =
        serde_json::from_str(&fixture("docker-backend-branch.json").to_json()).unwrap();
    // `verify` now also needs the Hyper-V check, which a WSL machine skips.
    v["steps"][3]["depends_on"] = json!(["hyperv-path"]);
    let p = parse_plan(&v.to_string()).unwrap();
    let mut facts = KnownFacts::default();
    facts.steps.insert("detect-backend".into(), ok());
    facts.facts.insert("docker.backend".into(), FactValue::Text("wsl2".into()));
    facts.steps.insert("wsl-path".into(), ok());
    let f = frontier(p.plan(), p.graph(), &facts);
    assert!(f.skipped.contains(&"verify".to_owned()), "{f:?}");
    assert!(f.complete);
}

#[test]
fn a_plan_without_edges_runs_in_the_order_written() {
    let p = fixture("restart-boundary.json");
    let mut facts = KnownFacts::default();
    assert_eq!(frontier(p.plan(), p.graph(), &facts).ready, ["check-feature"]);
    facts.steps.insert("check-feature".into(), ok());
    assert_eq!(frontier(p.plan(), p.graph(), &facts).ready, ["enable-feature"]);
    facts.steps.insert("enable-feature".into(), ok());
    assert_eq!(frontier(p.plan(), p.graph(), &facts).ready, ["confirm-feature"]);
}

/// The textbook case: step 3 changes, and steps 4, 5 and
/// 7 depend on it, while 6 does not.
fn chain_plan() -> serde_json::Value {
    let step = |n: u32| {
        json!({
            "id": format!("s{n}"), "title": format!("Step {n}"), "objective": "Check something.",
            "kind": "validation", "shell": {"kind": "pwsh"}, "commands": [{"text": format!("Write-Output {n}")}]
        })
    };
    json!({
        "schema_version": "1.0", "plan_id": "p", "task_id": "t",
        "target": {"id": "local", "kind": "local_windows"},
        "agent": {"name": "codex", "version": "1"},
        "steps": (1..=7).map(step).collect::<Vec<_>>(),
        "edges": [
            {"from": "s1", "to": "s2"}, {"from": "s2", "to": "s3"}, {"from": "s3", "to": "s4"},
            {"from": "s4", "to": "s5"}, {"from": "s2", "to": "s6"}, {"from": "s3", "to": "s7"}
        ]
    })
}

#[test]
fn changing_a_step_affects_it_and_everything_after_it() {
    let old = parse_plan(&chain_plan().to_string()).unwrap();
    let mut new = old.plan().clone();
    new.steps[2].commands = vec![Command { text: "Write-Output changed".into(), purpose: None }];
    let new = ValidPlan::revalidate(new, true).unwrap();
    let d = diff(old.plan(), new.plan(), new.graph());
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].fields, ["commands"]);
    assert!(d.changed[0].execution_relevant);
    // Reported in execution order; s6 hangs off s2, so it is untouched.
    assert_eq!(d.affected, ["s3", "s4", "s5", "s7"]);
}

#[test]
fn rewording_a_step_affects_nothing_that_runs() {
    let old = parse_plan(&chain_plan().to_string()).unwrap();
    let mut new = old.plan().clone();
    new.steps[2].title = "A better title".into();
    new.steps[2].objective = "Said more clearly.".into();
    let new = ValidPlan::revalidate(new, true).unwrap();
    let d = diff(old.plan(), new.plan(), new.graph());
    assert_eq!(d.changed[0].fields, ["objective", "title"]);
    assert!(!d.changed[0].execution_relevant);
    assert!(d.affected.is_empty());
}

#[test]
fn a_new_edge_condition_affects_its_target_and_what_follows() {
    let old = parse_plan(&chain_plan().to_string()).unwrap();
    let mut v = chain_plan();
    v["edges"][1]["when"] = json!({"fact": {"name": "ready", "equals": true}});
    let new = parse_plan(&v.to_string()).unwrap();
    let d = diff(old.plan(), new.plan(), new.graph());
    assert_eq!(d.edges_changed, ["s2 -> s3"]);
    assert_eq!(d.affected, ["s3", "s4", "s5", "s7"]);
}

#[test]
fn removing_a_step_is_reported_and_its_dependants_are_affected() {
    let old = parse_plan(&chain_plan().to_string()).unwrap();
    let mut v = chain_plan();
    v["steps"].as_array_mut().unwrap().remove(5); // s6
    v["edges"].as_array_mut().unwrap().remove(4); // s2 -> s6
    let new = parse_plan(&v.to_string()).unwrap();
    let d = diff(old.plan(), new.plan(), new.graph());
    assert_eq!(d.removed, ["s6"]);
    assert_eq!(d.edges_changed, ["s2 -> s6"]);
    assert!(d.affected.is_empty(), "nothing depended on s6");
}

#[test]
fn changing_the_plan_wide_assumptions_affects_every_step() {
    let old = parse_plan(&chain_plan().to_string()).unwrap();
    let mut v = chain_plan();
    v["environment_assumptions"] = json!([{"description": "PowerShell 7", "check": {"tool_version": {"tool": "pwsh", "satisfies": ">=7"}}}]);
    let new = parse_plan(&v.to_string()).unwrap();
    let d = diff(old.plan(), new.plan(), new.graph());
    assert_eq!(d.plan_fields, ["environment_assumptions"]);
    assert_eq!(d.affected.len(), 7);
}

#[test]
fn an_unchanged_plan_has_an_empty_diff() {
    let a = parse_plan(&chain_plan().to_string()).unwrap();
    let d = diff(a.plan(), a.plan(), a.graph());
    assert!(d.is_empty() && d.affected.is_empty());
}
