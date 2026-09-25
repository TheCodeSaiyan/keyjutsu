//! Evaluating conditions against what KeyJutsu actually knows.
//!
//! Evaluation is three-valued. A fact KeyJutsu has not collected is
//! *unknown*, never assumed true or false. `all` and `any` follow Kleene's
//! rules, so an unknown only matters when it could change the answer: `any`
//! with one true operand is true even if another is unknown. When the answer
//! itself is unknown, the caller is told which facts it needs, so it can go
//! and collect them instead of guessing.

use serde::Serialize;

use crate::model::{Condition, FactValue, OutcomeIs, ServiceState};
use crate::version::{Constraint, Version};

/// What happened to a step that has finished or been skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum StepResult {
    /// `exit_code` is `None` where the shell cannot report one.
    Succeeded {
        exit_code: Option<i64>,
    },
    Failed {
        exit_code: Option<i64>,
    },
    Skipped,
}

/// Where facts come from. Validation (Milestone 5) supplies real probes; tests
/// supply fixed values. `None` means "not known", never "false".
pub trait Facts {
    fn step_result(&self, step: &str) -> Option<StepResult>;
    fn fact(&self, name: &str) -> Option<FactValue>;
    fn tool_version(&self, tool: &str) -> Option<String>;
    fn path_exists(&self, path: &str) -> Option<bool>;
    fn service_state(&self, name: &str) -> Option<ServiceState>;
}

/// A fact needed to decide a condition that was not available.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, ts_rs::TS)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum Missing {
    StepResult(String),
    ExitCode(String),
    Fact(String),
    ToolVersion(String),
    Path(String),
    Service(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Truth {
    True,
    False,
    /// Could not be decided; these facts would settle it.
    Unknown(Vec<Missing>),
}

impl Truth {
    fn from_bool(b: bool) -> Self {
        if b { Truth::True } else { Truth::False }
    }
}

pub fn evaluate(condition: &Condition, facts: &dyn Facts) -> Truth {
    match condition {
        Condition::All(cs) => {
            let mut missing = Vec::new();
            for c in cs {
                match evaluate(c, facts) {
                    Truth::False => return Truth::False,
                    Truth::Unknown(m) => missing.extend(m),
                    Truth::True => {}
                }
            }
            unknown_or(missing, Truth::True)
        }
        Condition::Any(cs) => {
            let mut missing = Vec::new();
            for c in cs {
                match evaluate(c, facts) {
                    Truth::True => return Truth::True,
                    Truth::Unknown(m) => missing.extend(m),
                    Truth::False => {}
                }
            }
            unknown_or(missing, Truth::False)
        }
        Condition::Not(c) => match evaluate(c, facts) {
            Truth::True => Truth::False,
            Truth::False => Truth::True,
            u => u,
        },
        Condition::StepOutcome { step, is } => match facts.step_result(step) {
            None => Truth::Unknown(vec![Missing::StepResult(step.clone())]),
            Some(r) => Truth::from_bool(matches!(
                (r, is),
                (StepResult::Succeeded { .. }, OutcomeIs::Succeeded)
                    | (StepResult::Failed { .. }, OutcomeIs::Failed)
                    | (StepResult::Skipped, OutcomeIs::Skipped)
            )),
        },
        Condition::ExitCode { step, equals } => match facts.step_result(step) {
            None => Truth::Unknown(vec![Missing::StepResult(step.clone())]),
            // A skipped step has no exit code to compare, so it does not equal anything.
            Some(StepResult::Skipped) => Truth::False,
            Some(StepResult::Succeeded { exit_code } | StepResult::Failed { exit_code }) => match exit_code {
                None => Truth::Unknown(vec![Missing::ExitCode(step.clone())]),
                Some(code) => Truth::from_bool(code == *equals),
            },
        },
        Condition::Fact { name, equals } => match facts.fact(name) {
            None => Truth::Unknown(vec![Missing::Fact(name.clone())]),
            Some(actual) => Truth::from_bool(fact_equals(&actual, equals)),
        },
        Condition::ToolVersion { tool, satisfies } => match facts.tool_version(tool) {
            None => Truth::Unknown(vec![Missing::ToolVersion(tool.clone())]),
            Some(reported) => match (Constraint::parse(satisfies), Version::parse_lenient(&reported)) {
                (Ok(c), Some(v)) => Truth::from_bool(c.matches(&v)),
                // Plans are checked for valid constraints when parsed; a tool
                // reporting a version that is not a number is as good as unknown.
                _ => Truth::Unknown(vec![Missing::ToolVersion(tool.clone())]),
            },
        },
        Condition::PathExists { path } => match facts.path_exists(path) {
            None => Truth::Unknown(vec![Missing::Path(path.clone())]),
            Some(b) => Truth::from_bool(b),
        },
        Condition::ServiceState { name, state } => match facts.service_state(name) {
            None => Truth::Unknown(vec![Missing::Service(name.clone())]),
            Some(s) => Truth::from_bool(s == *state),
        },
    }
}

fn unknown_or(mut missing: Vec<Missing>, otherwise: Truth) -> Truth {
    if missing.is_empty() {
        otherwise
    } else {
        missing.sort();
        missing.dedup();
        Truth::Unknown(missing)
    }
}

/// Numbers compare by value (`1` equals `1.0`); strings compare exactly, and
/// a string never equals a number or a boolean.
fn fact_equals(actual: &FactValue, expected: &FactValue) -> bool {
    match (actual, expected) {
        (FactValue::Number(a), FactValue::Number(b)) => a == b,
        (FactValue::Text(a), FactValue::Text(b)) => a == b,
        (FactValue::Bool(a), FactValue::Bool(b)) => a == b,
        _ => false,
    }
}

/// Fixed facts for tests and for replaying a recorded session.
#[derive(Debug, Default, Clone)]
pub struct KnownFacts {
    pub steps: std::collections::HashMap<String, StepResult>,
    pub facts: std::collections::HashMap<String, FactValue>,
    pub tools: std::collections::HashMap<String, String>,
    pub paths: std::collections::HashMap<String, bool>,
    pub services: std::collections::HashMap<String, ServiceState>,
}

impl Facts for KnownFacts {
    fn step_result(&self, step: &str) -> Option<StepResult> {
        self.steps.get(step).copied()
    }
    fn fact(&self, name: &str) -> Option<FactValue> {
        self.facts.get(name).cloned()
    }
    fn tool_version(&self, tool: &str) -> Option<String> {
        self.tools.get(tool).cloned()
    }
    fn path_exists(&self, path: &str) -> Option<bool> {
        self.paths.get(path).copied()
    }
    fn service_state(&self, name: &str) -> Option<ServiceState> {
        self.services.get(name).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(name: &str, v: FactValue) -> Condition {
        Condition::Fact { name: name.into(), equals: v }
    }

    fn known(pairs: &[(&str, FactValue)]) -> KnownFacts {
        KnownFacts {
            facts: pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_missing_fact_is_unknown_and_named() {
        let c = fact("docker.backend", FactValue::Text("wsl2".into()));
        assert_eq!(
            evaluate(&c, &KnownFacts::default()),
            Truth::Unknown(vec![Missing::Fact("docker.backend".into())])
        );
        let not = Condition::Not(Box::new(c));
        assert!(
            matches!(evaluate(&not, &KnownFacts::default()), Truth::Unknown(_)),
            "not(unknown) is unknown"
        );
    }

    #[test]
    fn kleene_logic_decides_when_it_can() {
        let t = fact("a", FactValue::Bool(true));
        let f = fact("a", FactValue::Bool(false));
        let u = fact("missing", FactValue::Bool(true));
        let k = known(&[("a", FactValue::Bool(true))]);
        assert_eq!(evaluate(&Condition::Any(vec![u.clone(), t.clone()]), &k), Truth::True);
        assert_eq!(evaluate(&Condition::All(vec![u.clone(), f.clone()]), &k), Truth::False);
        assert!(matches!(evaluate(&Condition::All(vec![u.clone(), t]), &k), Truth::Unknown(_)));
        assert!(matches!(evaluate(&Condition::Any(vec![u, f]), &k), Truth::Unknown(_)));
    }

    #[test]
    fn facts_compare_by_type() {
        let k = known(&[("n", FactValue::Number(1.0)), ("s", FactValue::Text("1".into()))]);
        assert_eq!(evaluate(&fact("n", FactValue::Number(1.0)), &k), Truth::True);
        assert_eq!(evaluate(&fact("s", FactValue::Number(1.0)), &k), Truth::False, "\"1\" is not 1");
        assert_eq!(evaluate(&fact("s", FactValue::Text("1".into())), &k), Truth::True);
    }

    #[test]
    fn exit_codes_need_a_shell_that_reports_them() {
        let c = Condition::ExitCode { step: "a".into(), equals: 0 };
        let mut k = KnownFacts::default();
        k.steps.insert("a".into(), StepResult::Succeeded { exit_code: None });
        assert_eq!(evaluate(&c, &k), Truth::Unknown(vec![Missing::ExitCode("a".into())]));
        k.steps.insert("a".into(), StepResult::Succeeded { exit_code: Some(0) });
        assert_eq!(evaluate(&c, &k), Truth::True);
        k.steps.insert("a".into(), StepResult::Skipped);
        assert_eq!(evaluate(&c, &k), Truth::False);
    }

    #[test]
    fn tool_versions_use_the_constraint_grammar() {
        let c = Condition::ToolVersion { tool: "pwsh".into(), satisfies: ">=7.4 <8".into() };
        let mut k = KnownFacts::default();
        k.tools.insert("pwsh".into(), "7.6.6".into());
        assert_eq!(evaluate(&c, &k), Truth::True);
        k.tools.insert("pwsh".into(), "5.1.26100".into());
        assert_eq!(evaluate(&c, &k), Truth::False);
        k.tools.insert("pwsh".into(), "not installed".into());
        assert!(matches!(evaluate(&c, &k), Truth::Unknown(_)));
    }
}
