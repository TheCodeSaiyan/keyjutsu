//! Property tests: many generated inputs against the plan crate's promises.
//!
//! This is not coverage-guided fuzzing (that needs cargo-fuzz and is
//! Milestone 17's), but it covers the same promises §55 asks fuzzing to test:
//! hostile input is refused cleanly, never with a panic, and the same input
//! always gets the same answer.

#![allow(clippy::unwrap_used)] // Helpers outside #[test] functions may unwrap too.

use std::path::Path;
use std::sync::LazyLock;

use keyjutsu_plan::condition::Truth;
use keyjutsu_plan::model::{Condition, FactValue, OutcomeIs, ServiceState};
use keyjutsu_plan::version::{Constraint, Version};
use keyjutsu_plan::{KnownFacts, StepResult, ValidPlan, evaluate, frontier, parse_plan, parse_proposal};
use proptest::prelude::*;
use serde_json::Value;

static DOCKER: LazyLock<String> = LazyLock::new(|| {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/plan/v1/examples/valid/docker-backend-branch.json");
    std::fs::read_to_string(p).unwrap()
});

static DOCKER_PLAN: LazyLock<ValidPlan> = LazyLock::new(|| parse_plan(&DOCKER).unwrap());

fn scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        ".{0,12}".prop_map(Value::from),
        Just(serde_json::json!([])),
        Just(serde_json::json!({})),
    ]
}

/// Replace or delete the value at a random position in a JSON document.
fn mutate(doc: &mut Value, mut choice: usize, replacement: Value, delete: bool) {
    fn nodes(v: &Value) -> usize {
        1 + match v {
            Value::Object(m) => m.values().map(nodes).sum(),
            Value::Array(a) => a.iter().map(nodes).sum(),
            _ => 0,
        }
    }
    fn walk(v: &mut Value, choice: &mut usize, replacement: &Value, delete: bool) -> bool {
        let keys: Vec<String> = match v {
            Value::Object(m) => m.keys().cloned().collect(),
            _ => Vec::new(),
        };
        if let Value::Object(m) = v {
            for k in keys {
                if *choice == 0 {
                    if delete {
                        m.remove(&k);
                    } else {
                        m.insert(k, replacement.clone());
                    }
                    return true;
                }
                *choice -= 1;
                if walk(m.get_mut(&k).unwrap(), choice, replacement, delete) {
                    return true;
                }
            }
        } else if let Value::Array(a) = v {
            for item in a.iter_mut() {
                if *choice == 0 {
                    *item = replacement.clone();
                    return true;
                }
                *choice -= 1;
                if walk(item, choice, replacement, delete) {
                    return true;
                }
            }
        }
        false
    }
    choice %= nodes(doc).max(1);
    walk(doc, &mut choice, &replacement, delete);
}

fn condition() -> impl Strategy<Value = Condition> {
    let step = prop_oneof![Just("a".to_owned()), Just("b".to_owned()), Just("zz".to_owned())];
    let leaf = prop_oneof![
        (
            step.clone(),
            prop_oneof![Just(OutcomeIs::Succeeded), Just(OutcomeIs::Failed), Just(OutcomeIs::Skipped)]
        )
            .prop_map(|(step, is)| Condition::StepOutcome { step, is }),
        (step, -2i64..3).prop_map(|(step, equals)| Condition::ExitCode { step, equals }),
        (prop_oneof![Just("x"), Just("y")], any::<bool>())
            .prop_map(|(n, b)| Condition::Fact { name: n.into(), equals: FactValue::Bool(b) }),
        ".{0,10}".prop_map(|s| Condition::ToolVersion { tool: "pwsh".into(), satisfies: s }),
        Just(Condition::ServiceState { name: "docker".into(), state: ServiceState::Running }),
    ];
    leaf.prop_recursive(5, 40, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..4).prop_map(Condition::All),
            prop::collection::vec(inner.clone(), 1..4).prop_map(Condition::Any),
            inner.prop_map(|c| Condition::Not(Box::new(c))),
        ]
    })
}

fn some_facts() -> impl Strategy<Value = KnownFacts> {
    let result = prop_oneof![
        Just(None),
        Just(Some(StepResult::Succeeded { exit_code: Some(0) })),
        Just(Some(StepResult::Succeeded { exit_code: None })),
        Just(Some(StepResult::Failed { exit_code: Some(1) })),
        Just(Some(StepResult::Skipped)),
    ];
    (result.clone(), result, prop::option::of(any::<bool>()), prop::option::of("[0-9.]{1,8}")).prop_map(
        |(a, b, x, pwsh)| {
            let mut k = KnownFacts::default();
            if let Some(a) = a {
                k.steps.insert("a".into(), a);
            }
            if let Some(b) = b {
                k.steps.insert("b".into(), b);
            }
            if let Some(x) = x {
                k.facts.insert("x".into(), FactValue::Bool(x));
            }
            if let Some(v) = pwsh {
                k.tools.insert("pwsh".into(), v);
            }
            k
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, ..ProptestConfig::default() })]

    #[test]
    fn arbitrary_text_is_refused_cleanly(text in ".{0,400}") {
        let _ = parse_proposal(&text);
    }

    #[test]
    fn mutated_plans_never_panic_and_always_get_the_same_answer(
        choice in 0usize..400, replacement in scalar(), delete in any::<bool>()
    ) {
        let mut doc: Value = serde_json::from_str(&DOCKER).unwrap();
        mutate(&mut doc, choice, replacement, delete);
        let text = doc.to_string();
        let first = format!("{:?}", parse_proposal(&text).map(|p| p.to_json()));
        let second = format!("{:?}", parse_proposal(&text).map(|p| p.to_json()));
        prop_assert_eq!(first, second);
    }

    #[test]
    fn whatever_parses_serialises_to_something_that_parses_the_same(
        choice in 0usize..400, replacement in scalar(), delete in any::<bool>()
    ) {
        let mut doc: Value = serde_json::from_str(&DOCKER).unwrap();
        mutate(&mut doc, choice, replacement, delete);
        if let Ok(p) = parse_proposal(&doc.to_string()) {
            let again = parse_proposal(&p.to_json()).unwrap();
            prop_assert_eq!(again.plan(), p.plan());
        }
    }

    #[test]
    fn version_grammar_never_panics(constraint in ".{0,24}", version in ".{0,16}") {
        if let (Ok(c), Some(v)) = (Constraint::parse(&constraint), Version::parse_lenient(&version)) {
            let _ = c.matches(&v);
        }
    }

    #[test]
    fn a_version_satisfies_its_own_prefix_and_exact_range(parts in prop::collection::vec(0u64..50, 1..5)) {
        let text = parts.iter().map(u64::to_string).collect::<Vec<_>>().join(".");
        let v = Version::parse_lenient(&text).unwrap();
        prop_assert!(Constraint::parse(&text).unwrap().matches(&v));
        let range = format!(">={text} <={text}");
        prop_assert!(Constraint::parse(&range).unwrap().matches(&v));
    }

    #[test]
    fn double_negation_changes_nothing(c in condition(), facts in some_facts()) {
        let not_not = Condition::Not(Box::new(Condition::Not(Box::new(c.clone()))));
        prop_assert_eq!(evaluate(&not_not, &facts), evaluate(&c, &facts));
    }

    #[test]
    fn a_decided_condition_stays_decided_when_more_is_known(c in condition(), facts in some_facts()) {
        // Adding facts can settle an unknown, but never flips a true or false.
        let before = evaluate(&c, &facts);
        let mut more = facts.clone();
        more.steps.entry("a".into()).or_insert(StepResult::Succeeded { exit_code: Some(0) });
        more.steps.entry("b".into()).or_insert(StepResult::Skipped);
        more.facts.entry("x".into()).or_insert(FactValue::Bool(true));
        more.facts.entry("y".into()).or_insert(FactValue::Bool(false));
        more.tools.entry("pwsh".into()).or_insert("7.6.6".into());
        more.services.entry("docker".into()).or_insert(ServiceState::Running);
        let after = evaluate(&c, &more);
        match before {
            Truth::True | Truth::False => prop_assert_eq!(after, before),
            Truth::Unknown(_) => {}
        }
    }

    #[test]
    fn the_walk_never_contradicts_itself(
        detect in prop_oneof![Just(None), Just(Some(true)), Just(Some(false))],
        backend in prop_oneof![Just(None), Just(Some("wsl2")), Just(Some("hyperv"))],
        branch_done in any::<bool>(),
    ) {
        let p = &*DOCKER_PLAN;
        let mut k = KnownFacts::default();
        if let Some(ok) = detect {
            k.steps.insert("detect-backend".into(), if ok {
                StepResult::Succeeded { exit_code: Some(0) }
            } else {
                StepResult::Failed { exit_code: Some(1) }
            });
        }
        if let Some(b) = backend {
            k.facts.insert("docker.backend".into(), FactValue::Text(b.into()));
        }
        if branch_done
            && detect == Some(true)
            && let Some(b) = backend
        {
            let taken = if b == "wsl2" { "wsl-path" } else { "hyperv-path" };
            k.steps.insert(taken.into(), StepResult::Succeeded { exit_code: Some(0) });
        }
        let f = frontier(p.plan(), p.graph(), &k);
        for s in &f.ready {
            prop_assert!(!f.skipped.contains(s), "{} both ready and skipped", s);
            prop_assert!(!k.steps.contains_key(s), "{} already ran", s);
        }
        if f.halted_by.is_some() {
            prop_assert!(f.ready.is_empty());
        }
        prop_assert!(f.ready.len() <= 1, "this plan never has two ready steps: {:?}", f.ready);
    }
}
