//! Deciding what runs next.
//!
//! KeyJutsu, not the agent, owns control flow (§10). Given what has happened
//! so far, `frontier` says which steps are ready, which have been skipped
//! because no branch leads to them, and whether the plan has halted or
//! finished. It is pure: the same plan and the same facts always give the same
//! answer, which is what lets a crashed or rebooted session work out where it
//! was (§32, §52).
//!
//! The rules:
//!
//! - A step with no incoming edges is an entry and is always taken.
//! - Otherwise it is taken when at least one incoming edge was: its source
//!   succeeded and its condition, if any, holds. It is skipped once every
//!   incoming edge is decided and none was taken, which is how the untaken
//!   side of a branch disappears and both sides can join again afterwards.
//! - A step also waits for everything in its `depends_on`. If one of those was
//!   skipped, the step is skipped too, because what it depends on never
//!   happened.
//! - Any failure halts the plan. Nothing after a failure is ready, whatever the
//!   graph says (§2.3).
//! - A condition that cannot be decided leaves its step waiting and reports
//!   which facts are needed. It is never guessed.

use std::collections::HashMap;

use serde::Serialize;

use crate::condition::{Facts, Missing, StepResult, Truth, evaluate};
use crate::graph::PlanGraph;
use crate::model::{FactValue, Plan, ServiceState};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "plan/")]
pub struct Frontier {
    /// Steps that may run now, in execution order.
    pub ready: Vec<String>,
    /// Steps no branch leads to, in execution order.
    pub skipped: Vec<String>,
    /// The step whose failure stopped the plan.
    pub halted_by: Option<String>,
    /// Facts that would let waiting steps be decided.
    pub needs: Vec<Missing>,
    /// Every step has either run or been skipped.
    pub complete: bool,
}

impl Frontier {
    /// The single step to run next. V1 runs one machine-changing step at a
    /// time (§34), in the graph's deterministic order.
    pub fn next(&self) -> Option<&str> {
        self.ready.first().map(String::as_str)
    }
}

/// The facts as given, plus the skips worked out so far, so conditions can
/// ask whether a step was skipped.
struct WithSkips<'a> {
    inner: &'a dyn Facts,
    skipped: &'a HashMap<String, ()>,
}

impl Facts for WithSkips<'_> {
    fn step_result(&self, step: &str) -> Option<StepResult> {
        self.inner
            .step_result(step)
            .or_else(|| self.skipped.contains_key(step).then_some(StepResult::Skipped))
    }
    fn fact(&self, name: &str) -> Option<FactValue> {
        self.inner.fact(name)
    }
    fn tool_version(&self, tool: &str) -> Option<String> {
        self.inner.tool_version(tool)
    }
    fn path_exists(&self, path: &str) -> Option<bool> {
        self.inner.path_exists(path)
    }
    fn service_state(&self, name: &str) -> Option<ServiceState> {
        self.inner.service_state(name)
    }
}

pub fn frontier(plan: &Plan, graph: &PlanGraph, facts: &dyn Facts) -> Frontier {
    let ids = graph.ids();
    let mut out = Frontier::default();

    for &i in graph.order_indices() {
        if let Some(StepResult::Failed { .. }) = facts.step_result(&ids[i]) {
            out.halted_by = Some(ids[i].clone());
            return out;
        }
    }

    let mut skipped: HashMap<String, ()> = HashMap::new();
    let mut undecided = 0;
    for &i in graph.order_indices() {
        let id = &ids[i];
        if facts.step_result(id).is_some() {
            continue;
        }
        let view = WithSkips { inner: facts, skipped: &skipped };
        let result_of = |j: usize| view.step_result(&ids[j]);

        let incoming = graph.control_in(i);
        let mut taken = incoming.is_empty();
        let mut waiting = false;
        let mut needs = Vec::new();
        for &(src, edge) in incoming {
            match result_of(src) {
                None => waiting = true,
                Some(StepResult::Succeeded { .. }) => {
                    match edge.and_then(|e| plan.edges[e].when.as_ref()).map(|c| evaluate(c, &view)) {
                        None | Some(Truth::True) => taken = true,
                        Some(Truth::False) => {}
                        Some(Truth::Unknown(m)) => needs.extend(m),
                    }
                }
                Some(_) => {}
            }
        }
        let deps: Vec<Option<StepResult>> = graph.dependencies(i).iter().map(|&d| result_of(d)).collect();
        let deps_waiting = deps.iter().any(Option::is_none);
        let dep_skipped = deps.iter().any(|d| matches!(d, Some(StepResult::Skipped)));

        // A join waits until every path into it is decided, so a step after
        // two parallel branches does not start while one is still running.
        if waiting || deps_waiting {
            out.needs.extend(needs);
            undecided += 1;
        } else if taken && !dep_skipped {
            out.ready.push(id.clone());
            undecided += 1;
        } else if taken || needs.is_empty() {
            skipped.insert(id.clone(), ());
            out.skipped.push(id.clone());
        } else {
            out.needs.extend(needs);
            undecided += 1;
        }
    }
    out.needs.sort();
    out.needs.dedup();
    out.complete = undecided == 0;
    out
}
