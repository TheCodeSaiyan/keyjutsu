//! Every schema fixture, through the Rust parser. The Node check
//! (`pnpm schemas:check`) runs the same files through ajv; together they show
//! the Rust gate and the published schema agree on each one.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::path::{Path, PathBuf};

use keyjutsu_plan::hash::step_hashes;
use keyjutsu_plan::{PlanError, Problem, parse_plan, parse_proposal};

fn dir(kind: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/plan/v1/examples").join(kind)
}

fn fixtures(kind: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir(kind))
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap())
        })
        .collect();
    out.sort();
    assert!(!out.is_empty(), "no fixtures in {kind}");
    out
}

fn load(kind: &str, name: &str) -> String {
    std::fs::read_to_string(dir(kind).join(name)).unwrap()
}

#[test]
fn valid_fixtures_parse_as_proposals_and_as_plans() {
    for (name, text) in fixtures("valid") {
        parse_proposal(&text).unwrap_or_else(|e| panic!("{name}: {e} {e:?}"));
        parse_plan(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn schema_invalid_fixtures_are_refused_before_the_model_sees_them() {
    for (name, text) in fixtures("invalid") {
        match parse_proposal(&text) {
            Err(PlanError::Schema { violations }) => assert!(!violations.is_empty(), "{name}"),
            Err(PlanError::UnsupportedVersion { .. }) => assert_eq!(name, "future-major-version.json"),
            other => panic!("{name}: expected a schema refusal, got {other:?}"),
        }
    }
}

#[test]
fn an_agent_cannot_claim_readiness_but_a_stored_plan_may_record_it() {
    let text = load("invalid", "agent-claims-readiness.json");
    assert!(matches!(parse_proposal(&text), Err(PlanError::Schema { .. })));
    let stored = parse_plan(&text).unwrap();
    assert!(stored.plan().keyjutsu.is_some());
}

#[test]
fn schema_messages_never_quote_the_document() {
    for (name, text) in fixtures("invalid") {
        if let Err(PlanError::Schema { violations }) = parse_proposal(&text) {
            for v in violations {
                assert!(v.message.chars().count() <= 301, "{name}: {}", v.message);
                assert!(!v.message.contains("\"plan_id\""), "{name} leaks the plan: {}", v.message);
            }
        }
    }
    let Err(PlanError::Schema { violations }) =
        parse_proposal(&load("invalid", "agent-claims-readiness.json"))
    else {
        panic!()
    };
    assert!(violations[0].message.contains("only by KeyJutsu"), "{}", violations[0].message);
}

#[test]
fn a_future_version_is_refused_by_name_not_guessed_at() {
    let err = parse_proposal(&load("invalid", "future-major-version.json")).unwrap_err();
    assert_eq!(err, PlanError::UnsupportedVersion { found: "2.0".into(), supported: "1.0".into() });
}

#[test]
fn structure_invalid_fixtures_report_the_right_problem() {
    type Expectation = (&'static str, fn(&Problem) -> bool);
    let expected: &[Expectation] = &[
        ("bad-version-range.json", |p| matches!(p, Problem::BadVersionConstraint { .. })),
        (
            "condition-on-later-step.json",
            |p| matches!(p, Problem::ConditionOnLaterStep { step, .. } if step == "verify"),
        ),
        (
            "cycle.json",
            |p| matches!(p, Problem::Cycle { steps } if steps.first() == steps.last() && steps.len() > 2),
        ),
        ("duplicate-step-id.json", |p| matches!(p, Problem::DuplicateStepId { step } if step == "wsl-path")),
        ("edge-to-missing-step.json", |p| matches!(p, Problem::UnknownStep { step, .. } if step == "verfy")),
        (
            "hidden-bidi-override.json",
            |p| matches!(p, Problem::HiddenCharacter { step, code_point } if step == "list-logs" && code_point == "U+202E"),
        ),
        ("phases-out-of-order.json", |p| matches!(p, Problem::PhaseOrder { .. })),
        (
            "question-about-missing-step.json",
            |p| matches!(p, Problem::UnknownStep { place, step } if place == "question `which-desktop`" && step == "make-pfd"),
        ),
        (
            "question-assumes-a-missing-option.json",
            |p| matches!(p, Problem::AssumedOptionMissing { question } if question == "open-afterwards"),
        ),
        (
            "duplicate-question-id.json",
            |p| matches!(p, Problem::DuplicateQuestionId { question } if question == "which-desktop"),
        ),
        (
            "hidden-character-in-question.json",
            |p| matches!(p, Problem::HiddenCharacterInQuestion { question, code_point } if question == "which-desktop" && code_point == "U+202E"),
        ),
    ];
    let files = fixtures("structure-invalid");
    assert_eq!(files.len(), expected.len(), "every structure fixture needs an expectation");
    for (name, text) in files {
        let (_, check) = expected.iter().find(|(n, _)| *n == name).unwrap();
        match parse_proposal(&text) {
            Err(PlanError::Invalid { problems }) => {
                assert!(problems.iter().any(check), "{name}: {problems:?}");
            }
            other => panic!("{name}: expected a structural refusal, got {other:?}"),
        }
    }
}

#[test]
fn problems_are_all_reported_at_once() {
    // Two independent mistakes in one plan: both come back.
    let mut v: serde_json::Value =
        serde_json::from_str(&load("valid", "docker-backend-branch.json")).unwrap();
    v["edges"][2]["to"] = "nowhere".into();
    v["environment_assumptions"][0]["check"]["tool_version"]["satisfies"] = "7.*.1".into();
    let Err(PlanError::Invalid { problems }) = parse_proposal(&v.to_string()) else { panic!() };
    assert!(problems.iter().any(|p| matches!(p, Problem::UnknownStep { .. })));
    assert!(problems.iter().any(|p| matches!(p, Problem::BadVersionConstraint { .. })));
}

#[test]
fn parsing_is_deterministic_and_round_trips() {
    for (name, text) in fixtures("valid") {
        let a = parse_plan(&text).unwrap();
        let b = parse_plan(&text).unwrap();
        assert_eq!(a.plan(), b.plan(), "{name}");
        let json = a.to_json();
        let again = parse_plan(&json).unwrap_or_else(|e| panic!("{name}: our own output failed: {e}"));
        assert_eq!(again.plan(), a.plan(), "{name}");
        assert_eq!(again.to_json(), json, "{name}: serialisation is not stable");
    }
}

#[test]
fn key_order_in_the_input_does_not_matter() {
    let text = load("valid", "docker-backend-branch.json");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    // serde_json's map is ordered, so rebuild it with keys reversed.
    fn reverse(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(m) => {
                let mut out = serde_json::Map::new();
                for (k, v) in m.iter().rev() {
                    out.insert(k.clone(), reverse(v));
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(a) => serde_json::Value::Array(a.iter().map(reverse).collect()),
            other => other.clone(),
        }
    }
    let reversed = reverse(&v).to_string();
    assert_eq!(parse_plan(&reversed).unwrap().to_json(), parse_plan(&text).unwrap().to_json());
}

#[test]
fn the_graph_orders_steps_deterministically() {
    let p = parse_plan(&load("valid", "docker-backend-branch.json")).unwrap();
    let order: Vec<&str> = p.graph().topological_order().collect();
    assert_eq!(order, ["detect-backend", "wsl-path", "hyperv-path", "verify"]);
    let p = parse_plan(&load("valid", "restart-boundary.json")).unwrap();
    let order: Vec<&str> = p.graph().topological_order().collect();
    assert_eq!(order, ["check-feature", "enable-feature", "confirm-feature"]);
}

#[test]
fn oversized_and_non_json_input_is_refused() {
    let big = format!(
        "{{\"schema_version\":\"1.0\",\"pad\":\"{}\"}}",
        "x".repeat(keyjutsu_plan::parse::MAX_PLAN_BYTES)
    );
    assert!(matches!(parse_proposal(&big), Err(PlanError::TooLarge { .. })));
    assert!(matches!(parse_proposal("not json"), Err(PlanError::Json { .. })));
    assert!(matches!(parse_proposal("{}"), Err(PlanError::MissingVersion)));
    assert!(matches!(parse_proposal(r#"{"schema_version":1}"#), Err(PlanError::UnsupportedVersion { .. })));
    // Deep nesting hits serde_json's recursion limit rather than the stack.
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    assert!(matches!(parse_proposal(&deep), Err(PlanError::Json { .. })));
}

/// Questions are for the operator; approval never binds to them. A plan
/// without any is written exactly as before they existed, so a stored plan
/// or snapshot reads back unchanged, and adding them changes no step's hash.
#[test]
fn questions_change_no_step_hash_and_are_absent_when_there_are_none() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/check-a-service.json");
    let text = std::fs::read_to_string(path).unwrap();
    let plan = parse_plan(&text).unwrap();
    let hashes = step_hashes(plan.plan(), plan.graph());
    // Recorded with the plan model as it was before questions were added.
    assert_eq!(hashes["look"], "34dd709059fcac26b38393069d7891267e661dc91ea8f52120796733396a3728");
    assert_eq!(hashes["when"], "51089285f18fbd060aa30a0ebad6d060c121022396f6fc5c5fc6c39c37ca9bc4");
    assert!(!plan.to_json().contains("\"questions\""), "an empty list is left out");

    let mut asked: serde_json::Value = serde_json::from_str(&text).unwrap();
    asked["questions"] = serde_json::json!([
        {"id": "which", "step": "look", "text": "Which service?", "options": ["Winmgmt", "Spooler"]}
    ]);
    let asked = parse_proposal(&asked.to_string()).unwrap();
    assert_eq!(asked.plan().questions.len(), 1);
    assert_eq!(step_hashes(asked.plan(), asked.graph()), hashes);
}
