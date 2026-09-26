//! Techniques: a successful session made reusable.
//!
//! A Technique is a plan template with named parameters (`{{kj:service_name}}`
//! in its text), the environments it is known to have worked on, and where it
//! came from. It never runs because it worked before: using one produces an
//! ordinary draft plan, which is compared with the known-good environments
//! and validated on this machine, and then approved like any other. Each
//! revision is kept; nothing rewrites an earlier one or the sessions it came
//! from.

use std::collections::BTreeMap;

use keyjutsu_plan::ValidPlan;
use keyjutsu_plan::hash::{Drift, EnvironmentFingerprint, affected_by_drift};
use keyjutsu_plan::model::{Agent, Plan};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::history::SessionRecord;
use crate::store::Store;

pub const KIND: &str = "technique";
const EXPORT_KIND: &str = "keyjutsu.technique/1";

/// A parameter's value may match any pattern the author sets, but never
/// contains these: quotes, variables, statement separators, pipes,
/// redirection, backticks or line breaks. A value is data, not code.
const NEVER_IN_A_VALUE: &[char] =
    &['\'', '"', '$', ';', '|', '&', '<', '>', '`', '\r', '\n', '{', '}', '(', ')'];
pub const DEFAULT_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "technique/")]
pub struct Parameter {
    pub name: String,
    pub description: String,
    /// A regular expression each value must match in full.
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "technique/")]
pub struct TechniqueProvenance {
    /// The session it was promoted from; `None` once exported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub origin_session: Option<String>,
    pub agent: Agent,
    #[serde(default)]
    pub reviewers: Vec<Agent>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_validated_at: Option<String>,
    /// Environments it ran successfully on.
    #[serde(default)]
    pub known_good: Vec<EnvironmentFingerprint>,
    /// It came from another machine: an untrusted draft until approved here.
    #[serde(default)]
    pub imported: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "technique/")]
pub struct Technique {
    /// Lower-case, from the name: `restart-and-validate-windows-service`.
    pub id: String,
    pub name: String,
    pub description: String,
    pub revision: u32,
    pub parameters: Vec<Parameter>,
    /// The plan with `{{kj:parameter}}` placeholders and no KeyJutsu state.
    pub template: Plan,
    pub provenance: TechniqueProvenance,
}

/// `{{kj:name}}`: distinct from the `{{.Field}}` templates that tools such
/// as `docker --format` use in real commands.
pub fn placeholder(name: &str) -> String {
    format!("{{{{kj:{name}}}}}")
}

fn record_id(id: &str, revision: u32) -> String {
    format!("{id}-r{revision}")
}

fn slug(name: &str) -> String {
    let s: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let s: String = s.chars().take(64).collect();
    if s.is_empty() { "technique".into() } else { s }
}

/// Every string in the steps and edges, where parameters may appear.
fn each_string(v: &mut Value, f: &mut dyn FnMut(&mut String)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter_mut().for_each(|x| each_string(x, f)),
        Value::Object(o) => {
            o.iter_mut().filter(|(k, _)| k.as_str() != "id").for_each(|(_, x)| each_string(x, f))
        }
        _ => {}
    }
}

fn map_plan_strings(plan: &Plan, f: &mut dyn FnMut(&mut String)) -> Result<Plan, String> {
    let mut v = serde_json::to_value(plan).map_err(|e| e.to_string())?;
    for key in ["steps", "edges", "title"] {
        if let Some(part) = v.get_mut(key) {
            each_string(part, f);
        }
    }
    serde_json::from_value(v).map_err(|e| e.to_string())
}

fn check_value(p: &Parameter, value: &str) -> Result<(), String> {
    if let Some(c) = value.chars().find(|c| NEVER_IN_A_VALUE.contains(c)) {
        return Err(format!("{}: `{c}` is never allowed in a parameter value", p.name));
    }
    if let Some(c) = value.chars().find(|c| keyjutsu_plan::graph::is_hidden(*c)) {
        return Err(format!(
            "{}: U+{:04X} is never allowed in a parameter value: it is invisible or reorders the text",
            p.name,
            u32::from(c)
        ));
    }
    let re = regex::Regex::new(&format!("^(?:{})$", p.pattern.trim_start_matches('^').trim_end_matches('$')))
        .map_err(|e| format!("{}: its pattern is not valid: {e}", p.name))?;
    if !re.is_match(value) {
        return Err(format!("{}: `{value}` does not match {}", p.name, p.pattern));
    }
    Ok(())
}

/// A parameter to make of a value seen in the session.
#[derive(Debug, Clone)]
pub struct Promote<'a> {
    pub name: &'a str,
    pub description: &'a str,
    /// The literal value in the session's plan that becomes `{{kj:name}}`.
    pub value: &'a str,
    pub pattern: Option<&'a str>,
}

/// Make a successful session into revision 1 of a Technique.
pub fn promote(
    session: &SessionRecord,
    name: &str,
    description: &str,
    params: &[Promote<'_>],
    at: &str,
) -> Result<Technique, String> {
    if !session.succeeded() {
        return Err("only a session that completed can become a Technique".into());
    }
    let snap = session.snapshot()?;
    let mut template = snap.plan().clone();
    template.keyjutsu = None;
    let mut parameters = Vec::new();
    for p in params {
        if !p.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || p.name.is_empty() {
            return Err(format!("`{}` is not a parameter name: letters, digits and _ only", p.name));
        }
        let param = Parameter {
            name: p.name.to_owned(),
            description: p.description.to_owned(),
            pattern: p.pattern.unwrap_or(DEFAULT_PATTERN).to_owned(),
            default: Some(p.value.to_owned()),
        };
        check_value(&param, p.value)?;
        let placeholder = placeholder(p.name);
        let mut seen = 0;
        template = map_plan_strings(&template, &mut |s| {
            seen += s.matches(p.value).count();
            *s = s.replace(p.value, &placeholder);
        })?;
        if seen == 0 {
            return Err(format!(
                "`{}` does not appear in the plan, so it cannot become a parameter",
                p.value
            ));
        }
        parameters.push(param);
    }
    let reviewers = snap
        .plan()
        .keyjutsu
        .iter()
        .flat_map(|k| k.provenance.iter())
        .filter(|e| e.action == keyjutsu_plan::model::ProvenanceAction::Challenged)
        .filter_map(|e| e.actor.agent.clone())
        .collect();
    Ok(Technique {
        id: slug(name),
        name: name.to_owned(),
        description: description.to_owned(),
        revision: 1,
        parameters,
        template,
        provenance: TechniqueProvenance {
            origin_session: Some(session.id.clone()),
            agent: session.agent.clone(),
            reviewers,
            created_at: at.to_owned(),
            last_validated_at: Some(snap.sealed_at().to_owned()),
            known_good: snap.fingerprint().cloned().into_iter().collect(),
            imported: false,
        },
    })
}

/// A draft plan from a Technique and parameter values. Every value is
/// checked; every placeholder must be filled. The draft carries no approval
/// and no validation.
pub fn instantiate(t: &Technique, values: &BTreeMap<String, String>) -> Result<ValidPlan, String> {
    let mut plan = t.template.clone();
    for p in &t.parameters {
        let value =
            values.get(&p.name).or(p.default.as_ref()).ok_or_else(|| format!("{} needs a value", p.name))?;
        check_value(p, value)?;
        let placeholder = placeholder(&p.name);
        plan = map_plan_strings(&plan, &mut |s| *s = s.replace(&placeholder, value))?;
    }
    if let Some(unknown) = values.keys().find(|k| !t.parameters.iter().any(|p| &p.name == *k)) {
        return Err(format!("{} has no parameter `{unknown}`", t.name));
    }
    let left = serde_json::to_string(&plan).map_err(|e| e.to_string())?;
    if let Some(i) = left.find("{{kj:") {
        let rest: String = left[i..].chars().take(40).collect();
        return Err(format!("a placeholder was left unfilled: {rest}"));
    }
    plan.plan_id = format!("{}-r{}", t.id, t.revision);
    plan.keyjutsu = None;
    ValidPlan::revalidate(plan, false).map_err(|e| e.to_string())
}

/// How a Technique stands on this machine, before any validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "technique/")]
pub struct Fit {
    /// Differences from the closest known-good environment.
    pub drifts: Vec<Drift>,
    /// Steps whose past success says nothing about this machine.
    pub requires_revalidation: Vec<String>,
    /// It has never run here or anywhere known: everything must be validated.
    pub no_known_good: bool,
}

/// Compare this machine with the environments a Technique worked on.
pub fn fit(t: &Technique, plan: &ValidPlan, now: &EnvironmentFingerprint) -> Fit {
    let closest = t.provenance.known_good.iter().map(|k| k.drift(now)).min_by_key(Vec::len);
    match closest {
        None => Fit {
            drifts: Vec::new(),
            requires_revalidation: plan.graph().topological_order().map(str::to_owned).collect(),
            no_known_good: true,
        },
        Some(drifts) => Fit {
            requires_revalidation: affected_by_drift(plan.plan(), plan.graph(), &drifts),
            drifts,
            no_known_good: false,
        },
    }
}

pub fn save(store: &Store, t: &Technique) -> Result<(), String> {
    let id = record_id(&t.id, t.revision);
    if store.get::<Technique>(KIND, &id)?.is_some() {
        return Err(format!(
            "{} revision {} already exists; revisions are never rewritten",
            t.id, t.revision
        ));
    }
    store.put(KIND, &id, t)
}

/// Every revision of `id`, oldest first.
pub fn revisions(store: &Store, id: &str) -> Result<Vec<Technique>, String> {
    let prefix = format!("{id}-r");
    let mut out: Vec<Technique> = Vec::new();
    for rid in store
        .list(KIND)?
        .into_iter()
        .filter(|r| r.strip_prefix(&prefix).is_some_and(|n| n.parse::<u32>().is_ok()))
    {
        if let Some(t) = store.get(KIND, &rid)? {
            out.push(t);
        }
    }
    out.sort_by_key(|t| t.revision);
    Ok(out)
}

pub fn latest(store: &Store, id: &str) -> Result<Technique, String> {
    revisions(store, id)?.pop().ok_or_else(|| format!("there is no Technique `{id}`"))
}

/// The latest revision of every Technique.
pub fn list(store: &Store) -> Result<Vec<Technique>, String> {
    let mut ids: Vec<String> = store
        .list(KIND)?
        .into_iter()
        .filter_map(|r| r.rsplit_once("-r").map(|(id, _)| id.to_owned()))
        .collect();
    ids.dedup();
    ids.iter().map(|id| latest(store, id)).collect()
}

/// A new revision from an adapted template (after revalidation or an
/// agent's adaptation, and the operator's approval). The earlier revision
/// is kept as it was.
pub fn revise(
    store: &Store,
    t: &Technique,
    template: Plan,
    known_good: Option<EnvironmentFingerprint>,
    at: &str,
) -> Result<Technique, String> {
    let latest = latest(store, &t.id)?;
    let mut next = latest.clone();
    next.revision = latest.revision + 1;
    next.template = Plan { keyjutsu: None, ..template };
    next.provenance.last_validated_at = Some(at.to_owned());
    if let Some(k) = known_good {
        next.provenance.known_good.push(k);
    }
    save(store, &next)?;
    Ok(next)
}

/// For sharing: no originating session and no environments of this
/// machine, which could say more about it than the operator means to share.
pub fn export(t: &Technique) -> String {
    let mut shared = t.clone();
    shared.provenance.origin_session = None;
    shared.provenance.known_good.clear();
    shared.provenance.last_validated_at = None;
    serde_json::to_string_pretty(&serde_json::json!({ "kind": EXPORT_KIND, "technique": shared }))
        .unwrap_or_default()
}

/// Read a shared Technique. It is untrusted: its template is parsed and
/// checked against the schema here, and it arrives with no known-good
/// environment, so every step must be validated and approved on this machine.
pub fn import(text: &str) -> Result<Technique, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    if v.get("kind").and_then(Value::as_str) != Some(EXPORT_KIND) {
        return Err(format!("not a KeyJutsu Technique export (expected kind {EXPORT_KIND})"));
    }
    let mut t: Technique =
        serde_json::from_value(v["technique"].clone()).map_err(|e| format!("not a Technique: {e}"))?;
    t.id = slug(&t.id);
    for p in &t.parameters {
        if let Some(d) = &p.default {
            check_value(p, d)?;
        }
    }
    t.template.keyjutsu = None;
    ValidPlan::revalidate(t.template.clone(), true).map_err(|e| match e {
        keyjutsu_plan::PlanError::Invalid { problems } => format!(
            "its plan is not valid: {}",
            problems.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
        ),
        e => format!("its plan is not valid: {e}"),
    })?;
    t.provenance.imported = true;
    t.provenance.known_good.clear();
    t.provenance.last_validated_at = None;
    t.provenance.origin_session = None;
    Ok(t)
}
