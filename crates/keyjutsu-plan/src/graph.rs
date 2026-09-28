//! The plan as a graph, and everything wrong with it that the schema cannot
//! see.
//!
//! Control flow comes from `edges`. A plan with no edges runs its steps in the
//! order written. `depends_on` adds ordering requirements on top: a step waits
//! for every step it depends on. The graph used for ordering and cycle checks
//! is the union of both.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use serde::Serialize;

use crate::model::{Condition, Plan};
use crate::version::Constraint;

/// Something wrong with a plan's structure. All of them are collected, not
/// just the first, so an agent asked to revise a plan sees everything at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "plan/")]
pub enum Problem {
    DuplicateStepId {
        step: String,
    },
    DuplicatePhaseId {
        phase: String,
    },
    /// `place` says where the reference was, e.g. `edge detect -> verify`.
    UnknownStep {
        place: String,
        step: String,
    },
    UnknownTarget {
        step: String,
        target: String,
    },
    SelfEdge {
        step: String,
    },
    DuplicateEdge {
        from: String,
        to: String,
    },
    /// The steps on one cycle, in order, first repeated at the end.
    Cycle {
        steps: Vec<String>,
    },
    /// A condition asks about a step that cannot have run by the time the
    /// condition is evaluated.
    ConditionOnLaterStep {
        place: String,
        step: String,
    },
    StepInSeveralPhases {
        step: String,
    },
    StepInNoPhase {
        step: String,
    },
    /// A step in an earlier phase has to wait for one in a later phase.
    PhaseOrder {
        earlier: String,
        later: String,
    },
    BadVersionConstraint {
        place: String,
        detail: String,
    },
    StateForUnknownStep {
        step: String,
    },
    /// A command contains a character that is invisible or reorders the text
    /// around it, so what the operator reads is not what runs.
    HiddenCharacter {
        step: String,
        code_point: String,
    },
    DuplicateQuestionId {
        question: String,
    },
    /// `assumed` names an option the question does not have.
    AssumedOptionMissing {
        question: String,
    },
    /// A question, or one of its options, contains such a character: what
    /// the operator picks must be what the agent is told they picked.
    HiddenCharacterInQuestion {
        question: String,
        code_point: String,
    },
}

/// Characters that change how a command reads without being seen: controls,
/// bidirectional overrides and isolates, zero-width characters, the soft
/// hyphen and the byte-order mark. A command containing one could be approved
/// as one thing and run as another ("Trojan Source", CVE-2021-42574).
pub fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{00AD}' | '\u{061C}' | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}')
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::DuplicateStepId { step } => write!(f, "step id `{step}` is used more than once"),
            Problem::DuplicatePhaseId { phase } => write!(f, "phase id `{phase}` is used more than once"),
            Problem::UnknownStep { place, step } => {
                write!(f, "{place} refers to step `{step}`, which does not exist")
            }
            Problem::UnknownTarget { step, target } => {
                write!(f, "step `{step}` names target `{target}`, but the plan's target is different")
            }
            Problem::SelfEdge { step } => write!(f, "step `{step}` has an edge to itself"),
            Problem::DuplicateEdge { from, to } => {
                write!(f, "the edge `{from}` -> `{to}` appears more than once")
            }
            Problem::Cycle { steps } => write!(f, "the steps form a cycle: {}", steps.join(" -> ")),
            Problem::ConditionOnLaterStep { place, step } => {
                write!(f, "{place} depends on step `{step}`, which cannot have run by then")
            }
            Problem::StepInSeveralPhases { step } => write!(f, "step `{step}` is in more than one phase"),
            Problem::StepInNoPhase { step } => write!(f, "the plan has phases but step `{step}` is in none"),
            Problem::PhaseOrder { earlier, later } => {
                write!(f, "step `{earlier}` is in an earlier phase than `{later}` but has to wait for it")
            }
            Problem::BadVersionConstraint { place, detail } => write!(f, "{place}: {detail}"),
            Problem::StateForUnknownStep { step } => {
                write!(f, "KeyJutsu state is recorded for step `{step}`, which does not exist")
            }
            Problem::HiddenCharacter { step, code_point } => write!(
                f,
                "step `{step}` has a command containing {code_point}, which is invisible or reorders the text, so the command would not read as it runs"
            ),
            Problem::DuplicateQuestionId { question } => {
                write!(f, "question id `{question}` is used more than once")
            }
            Problem::AssumedOptionMissing { question } => {
                write!(f, "question `{question}` says the plan follows an option it does not have")
            }
            Problem::HiddenCharacterInQuestion { question, code_point } => write!(
                f,
                "question `{question}` contains {code_point}, which is invisible or reorders the text, so it would not read as it is"
            ),
        }
    }
}

/// A structurally valid plan's graph. Indices are positions in `plan.steps`.
#[derive(Debug, Clone)]
pub struct PlanGraph {
    ids: Vec<String>,
    index: HashMap<String, usize>,
    /// Control-flow predecessors: (source index, edge position or None when implicit).
    control_in: Vec<Vec<(usize, Option<usize>)>>,
    depends_on: Vec<Vec<usize>>,
    /// Union of control flow and dependencies, for ordering.
    successors: Vec<Vec<usize>>,
    order: Vec<usize>,
}

impl PlanGraph {
    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    /// Steps in execution order: every step after all of its predecessors,
    /// ties broken by the order the plan lists them in, so the result never
    /// depends on anything but the plan.
    pub fn topological_order(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(|&i| self.ids[i].as_str())
    }

    pub(crate) fn order_indices(&self) -> &[usize] {
        &self.order
    }

    pub(crate) fn control_in(&self, i: usize) -> &[(usize, Option<usize>)] {
        &self.control_in[i]
    }

    pub(crate) fn dependencies(&self, i: usize) -> &[usize] {
        &self.depends_on[i]
    }

    /// Every step that transitively follows `id`, excluding it.
    pub fn descendants(&self, id: &str) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let Some(start) = self.index_of(id) else { return seen };
        let mut stack = self.successors[start].clone();
        while let Some(i) = stack.pop() {
            if seen.insert(self.ids[i].clone()) {
                stack.extend(&self.successors[i]);
            }
        }
        seen
    }

    fn ancestors_of(&self, target: usize) -> Vec<bool> {
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); self.ids.len()];
        for (from, tos) in self.successors.iter().enumerate() {
            for &to in tos {
                preds[to].push(from);
            }
        }
        let mut seen = vec![false; self.ids.len()];
        let mut stack = preds[target].clone();
        while let Some(i) = stack.pop() {
            if !seen[i] {
                seen[i] = true;
                stack.extend(&preds[i]);
            }
        }
        seen
    }
}

/// Build the graph and report every structural problem.
pub fn analyse(plan: &Plan) -> Result<PlanGraph, Vec<Problem>> {
    let mut problems = Vec::new();

    let mut index = HashMap::new();
    let mut ids = Vec::new();
    for (i, step) in plan.steps.iter().enumerate() {
        if index.insert(step.id.clone(), i).is_some() {
            problems.push(Problem::DuplicateStepId { step: step.id.clone() });
        }
        ids.push(step.id.clone());
    }
    let lookup = |place: String, id: &str, problems: &mut Vec<Problem>| -> Option<usize> {
        let found = index.get(id).copied();
        if found.is_none() {
            problems.push(Problem::UnknownStep { place, step: id.to_owned() });
        }
        found
    };

    let n = plan.steps.len();
    let mut control_in: Vec<Vec<(usize, Option<usize>)>> = vec![Vec::new(); n];
    let mut depends_on: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); n];

    if plan.edges.is_empty() {
        for i in 1..n {
            control_in[i].push((i - 1, None));
            successors[i - 1].push(i);
        }
    } else {
        let mut seen_edges = BTreeSet::new();
        for (e, edge) in plan.edges.iter().enumerate() {
            let place = format!("edge `{}` -> `{}`", edge.from, edge.to);
            let from = lookup(place.clone(), &edge.from, &mut problems);
            let to = lookup(place, &edge.to, &mut problems);
            if edge.from == edge.to {
                problems.push(Problem::SelfEdge { step: edge.from.clone() });
                continue;
            }
            if !seen_edges.insert((edge.from.clone(), edge.to.clone())) {
                problems.push(Problem::DuplicateEdge { from: edge.from.clone(), to: edge.to.clone() });
            }
            if let (Some(f), Some(t)) = (from, to) {
                control_in[t].push((f, Some(e)));
                successors[f].push(t);
            }
        }
    }
    for (i, step) in plan.steps.iter().enumerate() {
        for dep in &step.depends_on {
            if let Some(d) = lookup(format!("step `{}` depends_on", step.id), dep, &mut problems) {
                if d == i {
                    problems.push(Problem::SelfEdge { step: step.id.clone() });
                } else {
                    depends_on[i].push(d);
                    successors[d].push(i);
                }
            }
        }
        if let Some(target) = &step.target_id
            && *target != plan.target.id
        {
            problems.push(Problem::UnknownTarget { step: step.id.clone(), target: target.clone() });
        }
    }

    // Kahn's algorithm, always taking the earliest-listed ready step.
    let mut indegree: Vec<usize> = vec![0; n];
    for tos in &successors {
        for &t in tos {
            indegree[t] += 1;
        }
    }
    let mut ready: BTreeSet<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(i) = ready.pop_first() {
        order.push(i);
        for &t in &successors[i] {
            indegree[t] -= 1;
            if indegree[t] == 0 {
                ready.insert(t);
            }
        }
    }
    if order.len() < n
        && let Some(cycle) = find_cycle(&successors, &ids, &order)
    {
        problems.push(Problem::Cycle { steps: cycle });
    }

    let graph = PlanGraph { ids, index: index.clone(), control_in, depends_on, successors, order };

    // Conditions may only ask about steps that have finished by the time they
    // are evaluated. Only meaningful on an acyclic graph.
    if graph.order.len() == n {
        for (i, step) in plan.steps.iter().enumerate() {
            let ancestors = graph.ancestors_of(i);
            for c in &step.preconditions {
                check_condition_steps(
                    c,
                    &format!("a precondition of `{}`", step.id),
                    &ancestors,
                    None,
                    &graph,
                    &mut problems,
                );
            }
        }
        for edge in &plan.edges {
            if let (Some(when), Some(&from)) = (&edge.when, graph.index.get(&edge.from)) {
                // On an edge, the source step itself has finished too.
                let ancestors = graph.ancestors_of(from);
                let place = format!("the condition on edge `{}` -> `{}`", edge.from, edge.to);
                check_condition_steps(when, &place, &ancestors, Some(from), &graph, &mut problems);
            }
        }
        check_phases(plan, &graph, &mut problems);
    }

    // Plan-wide conditions (assumptions) are checked before anything runs,
    // so they may not ask about steps at all.
    for a in &plan.environment_assumptions {
        for s in a.check.referenced_steps() {
            problems.push(Problem::ConditionOnLaterStep {
                place: "an environment assumption".into(),
                step: s.to_owned(),
            });
        }
    }

    check_versions(plan, &mut problems);
    check_hidden_characters(plan, &mut problems);
    check_questions(plan, &index, &mut problems);

    if let Some(state) = &plan.keyjutsu {
        for step in state.steps.keys() {
            if !index.contains_key(step) {
                problems.push(Problem::StateForUnknownStep { step: step.clone() });
            }
        }
    }

    if problems.is_empty() { Ok(graph) } else { Err(problems) }
}

fn check_condition_steps(
    c: &Condition,
    place: &str,
    ancestors: &[bool],
    also_done: Option<usize>,
    graph: &PlanGraph,
    problems: &mut Vec<Problem>,
) {
    for s in c.referenced_steps() {
        match graph.index_of(s) {
            None => problems.push(Problem::UnknownStep { place: place.to_owned(), step: s.to_owned() }),
            Some(j) if ancestors[j] || Some(j) == also_done => {}
            Some(_) => {
                problems.push(Problem::ConditionOnLaterStep { place: place.to_owned(), step: s.to_owned() })
            }
        }
    }
}

fn check_phases(plan: &Plan, graph: &PlanGraph, problems: &mut Vec<Problem>) {
    if plan.phases.is_empty() {
        return;
    }
    let mut phase_of: HashMap<&str, usize> = HashMap::new();
    let mut phase_ids = BTreeSet::new();
    for (p, phase) in plan.phases.iter().enumerate() {
        if !phase_ids.insert(phase.id.as_str()) {
            problems.push(Problem::DuplicatePhaseId { phase: phase.id.clone() });
        }
        for s in &phase.steps {
            if graph.index_of(s).is_none() {
                problems
                    .push(Problem::UnknownStep { place: format!("phase `{}`", phase.id), step: s.clone() });
            } else if phase_of.insert(s, p).is_some() {
                problems.push(Problem::StepInSeveralPhases { step: s.clone() });
            }
        }
    }
    for id in graph.ids() {
        if !phase_of.contains_key(id.as_str()) {
            problems.push(Problem::StepInNoPhase { step: id.clone() });
        }
    }
    for (from, tos) in graph.successors.iter().enumerate() {
        for &to in tos {
            let (a, b) = (&graph.ids[from], &graph.ids[to]);
            if let (Some(pa), Some(pb)) = (phase_of.get(a.as_str()), phase_of.get(b.as_str()))
                && pa > pb
            {
                problems.push(Problem::PhaseOrder { earlier: b.clone(), later: a.clone() });
            }
        }
    }
}

fn check_hidden_characters(plan: &Plan, problems: &mut Vec<Problem>) {
    for step in &plan.steps {
        let recovery = step.recovery.iter().flat_map(|r| &r.commands);
        let texts = step.commands.iter().chain(&step.visible_validation).chain(recovery);
        if let Some(c) = texts.flat_map(|c| c.text.chars()).find(|c| is_hidden(*c)) {
            problems.push(Problem::HiddenCharacter {
                step: step.id.clone(),
                code_point: format!("U+{:04X}", u32::from(c)),
            });
        }
    }
}

fn check_questions(plan: &Plan, steps: &HashMap<String, usize>, problems: &mut Vec<Problem>) {
    let mut seen = BTreeSet::new();
    for q in &plan.questions {
        if !seen.insert(q.id.as_str()) {
            problems.push(Problem::DuplicateQuestionId { question: q.id.clone() });
        }
        if let Some(step) = &q.step
            && !steps.contains_key(step)
        {
            problems.push(Problem::UnknownStep { place: format!("question `{}`", q.id), step: step.clone() });
        }
        if q.assumed.is_some_and(|i| usize::from(i) >= q.options.len()) {
            problems.push(Problem::AssumedOptionMissing { question: q.id.clone() });
        }
        let texts = std::iter::once(&q.text).chain(&q.options);
        if let Some(c) = texts.flat_map(|t| t.chars()).find(|c| is_hidden(*c)) {
            problems.push(Problem::HiddenCharacterInQuestion {
                question: q.id.clone(),
                code_point: format!("U+{:04X}", u32::from(c)),
            });
        }
    }
}

fn check_versions(plan: &Plan, problems: &mut Vec<Problem>) {
    let mut check = |place: String, text: &str| {
        if let Err(e) = Constraint::parse(text) {
            problems.push(Problem::BadVersionConstraint { place, detail: e.to_string() });
        }
    };
    for r in &plan.requirements {
        if let Some(v) = &r.version {
            check(format!("requirement `{}`", r.name), v);
        }
    }
    for a in &plan.environment_assumptions {
        for v in a.check.version_constraints() {
            check("an environment assumption".into(), v);
        }
    }
    for step in &plan.steps {
        let place = format!("step `{}`", step.id);
        if let Some(v) = step.shell.as_ref().and_then(|s| s.version.as_ref()) {
            check(place.clone(), v);
        }
        for r in &step.tool_requirements {
            if let Some(v) = &r.version {
                check(place.clone(), v);
            }
        }
        for c in &step.preconditions {
            for v in c.version_constraints() {
                check(place.clone(), v);
            }
        }
    }
    for edge in &plan.edges {
        if let Some(when) = &edge.when {
            for v in when.version_constraints() {
                check(format!("edge `{}` -> `{}`", edge.from, edge.to), v);
            }
        }
    }
}

/// One cycle among the steps Kahn's algorithm could not order.
fn find_cycle(successors: &[Vec<usize>], ids: &[String], ordered: &[usize]) -> Option<Vec<String>> {
    let mut done = vec![false; ids.len()];
    for &i in ordered {
        done[i] = true;
    }
    // 0 unvisited, 1 on the current path, 2 finished.
    let mut mark = vec![0u8; ids.len()];
    for start in (0..ids.len()).filter(|&i| !done[i]) {
        let mut path = Vec::new();
        if let Some(c) = dfs(start, successors, &done, &mut mark, &mut path) {
            return Some(c.into_iter().map(|i| ids[i].clone()).collect());
        }
    }
    None
}

fn dfs(
    i: usize,
    succ: &[Vec<usize>],
    done: &[bool],
    mark: &mut [u8],
    path: &mut Vec<usize>,
) -> Option<Vec<usize>> {
    if mark[i] == 1 {
        let start = path.iter().position(|&p| p == i)?;
        let mut cycle = path[start..].to_vec();
        cycle.push(i);
        return Some(cycle);
    }
    if mark[i] == 2 || done[i] {
        return None;
    }
    mark[i] = 1;
    path.push(i);
    for &t in &succ[i] {
        if let Some(c) = dfs(t, succ, done, mark, path) {
            return Some(c);
        }
    }
    path.pop();
    mark[i] = 2;
    None
}
