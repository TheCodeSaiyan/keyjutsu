//! What changed between two versions of a plan, and what that change touches.
//!
//! The shape of the answer: "Step 3 changed. Affected downstream assumptions: steps 4, 5 and 7.
//! Status: revalidation required." A change to what a step *does* makes it and
//! every step after it in the graph need revalidation. A change to how it is
//! *described* (title, objective, reason) does not, because nothing that runs
//! depends on wording. Approval will build on exactly this list.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::graph::PlanGraph;
use crate::model::{Plan, Step};

use crate::hash::DESCRIPTIVE_FIELDS as DESCRIPTIVE;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "plan/")]
pub struct StepChange {
    pub step: String,
    /// Names of the fields that differ, sorted.
    pub fields: Vec<String>,
    /// False when only descriptive fields changed.
    pub execution_relevant: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, ts_rs::TS)]
#[ts(export, export_to = "plan/")]
pub struct PlanDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<StepChange>,
    /// Edges added, removed or given a different condition, as `from -> to`.
    pub edges_changed: Vec<String>,
    /// Plan-level fields that differ, other than steps and edges.
    pub plan_fields: Vec<String>,
    /// Steps in the new plan that need revalidation: those added or changed
    /// in a way that affects execution, whose control flow changed, or that
    /// follow any of those in the graph.
    pub affected: Vec<String>,
}

impl PlanDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && self.edges_changed.is_empty()
            && self.plan_fields.is_empty()
    }
}

fn fields(step: &Step) -> Map<String, Value> {
    match serde_json::to_value(step) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn edge_map(plan: &Plan) -> std::collections::BTreeMap<(String, String), Value> {
    plan.edges
        .iter()
        .map(|e| ((e.from.clone(), e.to.clone()), serde_json::to_value(&e.when).unwrap_or(Value::Null)))
        .collect()
}

/// Compare `old` with `new`. `new_graph` must be `new`'s graph.
pub fn diff(old: &Plan, new: &Plan, new_graph: &PlanGraph) -> PlanDiff {
    let mut d = PlanDiff::default();
    let mut roots: BTreeSet<String> = BTreeSet::new();

    for step in &new.steps {
        match old.step(&step.id) {
            None => {
                d.added.push(step.id.clone());
                roots.insert(step.id.clone());
            }
            Some(before) if before != step => {
                let (a, b) = (fields(before), fields(step));
                let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
                let changed: Vec<String> =
                    keys.into_iter().filter(|k| a.get(*k) != b.get(*k)).map(|k| k.to_string()).collect();
                let execution_relevant = changed.iter().any(|f| !DESCRIPTIVE.contains(&f.as_str()));
                if execution_relevant {
                    roots.insert(step.id.clone());
                }
                d.changed.push(StepChange { step: step.id.clone(), fields: changed, execution_relevant });
            }
            Some(_) => {}
        }
    }
    for step in &old.steps {
        if new.step(&step.id).is_none() {
            d.removed.push(step.id.clone());
        }
    }

    // Control flow. With no edges a plan runs in listed order, so in that
    // case reordering is a control-flow change too.
    let (old_edges, new_edges) = (edge_map(old), edge_map(new));
    let implicit = |p: &Plan| p.edges.is_empty();
    if implicit(old) || implicit(new) {
        let order = |p: &Plan| p.steps.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        if implicit(old) != implicit(new) || order(old) != order(new) {
            d.edges_changed.push("step order".into());
            // Every step whose predecessor changed is affected.
            let old_order = order(old);
            for (i, id) in order(new).iter().enumerate() {
                let prev_new = i.checked_sub(1).map(|j| &new.steps[j].id);
                let prev_old = old_order
                    .iter()
                    .position(|o| o == id)
                    .and_then(|j| j.checked_sub(1))
                    .map(|j| &old_order[j]);
                if prev_new != prev_old {
                    roots.insert(id.clone());
                }
            }
        }
    }
    let keys: BTreeSet<&(String, String)> = old_edges.keys().chain(new_edges.keys()).collect();
    for key in keys {
        if old_edges.get(key) != new_edges.get(key) {
            d.edges_changed.push(format!("{} -> {}", key.0, key.1));
            if new.step(&key.1).is_some() {
                roots.insert(key.1.clone());
            }
        }
    }
    // Steps that followed a removed step lose something they relied on.
    for step in &new.steps {
        let lost_dependency = step.depends_on.iter().any(|dep| d.removed.contains(dep));
        if lost_dependency {
            roots.insert(step.id.clone());
        }
    }

    let (a, b) =
        (serde_json::to_value(old).unwrap_or_default(), serde_json::to_value(new).unwrap_or_default());
    if let (Value::Object(a), Value::Object(b)) = (a, b) {
        let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        d.plan_fields = keys
            .into_iter()
            .filter(|k| !matches!(k.as_str(), "steps" | "edges" | "keyjutsu") && a.get(*k) != b.get(*k))
            .map(|k| k.to_string())
            .collect();
    }

    // What every step relies on: changing it puts the whole plan in question.
    let plan_wide = ["target", "requirements", "environment_assumptions", "schema_version"];
    if d.plan_fields.iter().any(|f| plan_wide.contains(&f.as_str())) {
        roots.extend(new.steps.iter().map(|s| s.id.clone()));
    }

    let mut affected: BTreeSet<String> = BTreeSet::new();
    for root in &roots {
        affected.insert(root.clone());
        affected.extend(new_graph.descendants(root));
    }
    // Report in execution order, which is how an operator reads a plan.
    d.affected =
        new_graph.topological_order().filter(|id| affected.contains(*id)).map(str::to_owned).collect();
    d
}
