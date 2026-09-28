//! Step hashes, snapshot hashes and environment fingerprints.
//!
//! A **step hash** is what an approval binds to. It covers everything about a
//! step that affects what it runs (every field except its title, objective
//! and reason), the plan-wide context every step relies on (target,
//! requirements, environment assumptions, schema version), and, crucially,
//! the hashes of the steps before it and the conditions on the edges leading
//! in. So changing step 3 changes the hashes of every step after it, and their
//! approvals lapse without anyone having to remember to withdraw them. This is
//! the same "affected" set `diff` reports; a property test holds the two
//! together.
//!
//! A **snapshot hash** covers the whole record: the plan as approved (wording
//! included), the approvals, the environment fingerprint and when it was
//! sealed. It answers "what exact plan was executed?" and detects any edit to
//! a stored snapshot.
//!
//! Both are SHA-256 over RFC 8785 canonical JSON ([`crate::canonical`]), each
//! prefixed with a `kind` naming the hash's own format version, so a hash of
//! one kind can never be passed off as another.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::canonical::canonicalise;
use crate::graph::PlanGraph;
use crate::model::Plan;

/// Fields of a step that describe it without affecting what runs.
pub const DESCRIPTIVE_FIELDS: &[&str] = &["title", "objective", "reason"];

const STEP_HASH_KIND: &str = "keyjutsu.step-hash/1";
const FINGERPRINT_KIND: &str = "keyjutsu.fingerprint/1";

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of the canonical form of `value`.
pub fn hash_value(value: &Value) -> String {
    sha256_hex(canonicalise(value).as_bytes())
}

/// Every step's hash, keyed by step id.
pub fn step_hashes(plan: &Plan, graph: &PlanGraph) -> BTreeMap<String, String> {
    let plan_context = json!({
        "schema_version": plan.schema_version,
        "target": plan.target,
        "requirements": plan.requirements,
        "environment_assumptions": plan.environment_assumptions,
    });
    let mut hashes: BTreeMap<String, String> = BTreeMap::new();
    for id in graph.topological_order() {
        let Some(step) = plan.step(id) else { continue };
        let mut body = serde_json::to_value(step).unwrap_or(Value::Null);
        if let Value::Object(map) = &mut body {
            for field in DESCRIPTIVE_FIELDS {
                map.remove(*field);
            }
        }
        let index = graph.index_of(id).unwrap_or_default();
        let mut incoming: Vec<Value> = graph
            .control_in(index)
            .iter()
            .map(|&(src, edge)| {
                let from = &graph.ids()[src];
                json!({
                    "from": from,
                    "from_hash": hashes.get(from),
                    "when": edge.and_then(|e| plan.edges[e].when.as_ref()),
                })
            })
            .collect();
        incoming.sort_by_key(|v| v["from"].as_str().unwrap_or_default().to_owned());
        let depends_on: BTreeMap<&String, Option<&String>> =
            step.depends_on.iter().map(|d| (d, hashes.get(d))).collect();
        let input = json!({
            "kind": STEP_HASH_KIND,
            "plan": plan_context,
            "step": body,
            "incoming": incoming,
            "depends_on": depends_on,
        });
        hashes.insert(id.to_owned(), hash_value(&input));
    }
    hashes
}

/// The state of the machine an approval was given against. Compared later to
/// find drift. Collected by `keyjutsu-core`; this crate only holds and
/// compares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct EnvironmentFingerprint {
    /// For example `Windows 11 Pro 25H2`.
    pub os: String,
    /// For example `26200.9457`.
    pub build: String,
    pub architecture: String,
    pub shells: Vec<FingerprintEntry>,
    /// Executables the plan names, as they resolved on this machine.
    pub tools: Vec<FingerprintEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "plan/")]
pub struct FingerprintEntry {
    pub name: String,
    /// `None` when it could not be found.
    pub path: Option<String>,
    /// `None` when it was not detected.
    pub version: Option<String>,
}

/// One difference between two fingerprints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "plan/")]
pub struct Drift {
    /// `os`, `build`, `architecture`, `shell:<name>` or `tool:<name>`.
    pub what: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl EnvironmentFingerprint {
    pub fn hash(&self) -> String {
        let mut sorted = self.clone();
        sorted.shells.sort();
        sorted.tools.sort();
        hash_value(&json!({ "kind": FINGERPRINT_KIND, "fingerprint": sorted }))
    }

    /// What differs between `self` (then) and `now`.
    pub fn drift(&self, now: &EnvironmentFingerprint) -> Vec<Drift> {
        let mut out = Vec::new();
        for (what, a, b) in [
            ("os", &self.os, &now.os),
            ("build", &self.build, &now.build),
            ("architecture", &self.architecture, &now.architecture),
        ] {
            if a != b {
                out.push(Drift { what: what.into(), before: Some(a.clone()), after: Some(b.clone()) });
            }
        }
        for (prefix, then, current) in
            [("shell", &self.shells, &now.shells), ("tool", &self.tools, &now.tools)]
        {
            let describe = |e: &FingerprintEntry| match (&e.path, &e.version) {
                (None, _) => "not found".to_owned(),
                (Some(p), None) => p.clone(),
                (Some(p), Some(v)) => format!("{v} at {p}"),
            };
            let names: std::collections::BTreeSet<&String> =
                then.iter().chain(current.iter()).map(|e| &e.name).collect();
            for name in names {
                let a = then.iter().find(|e| &e.name == name);
                let b = current.iter().find(|e| &e.name == name);
                let same = match (a, b) {
                    (Some(a), Some(b)) => a == b || same_store_app(a, b),
                    (a, b) => a == b,
                };
                if !same {
                    out.push(Drift {
                        what: format!("{prefix}:{name}"),
                        before: a.map(describe),
                        after: b.map(describe),
                    });
                }
            }
        }
        out
    }
}

/// Whether two entries are one Store app, found once through its App
/// Execution Alias (`…\Microsoft\WindowsApps\pwsh.exe`) and once in its
/// package folder (`…\WindowsApps\<package>\pwsh.exe`). Which is found first
/// depends on PATH, and so on how KeyJutsu was started: PowerShell 7 puts its
/// own folder first for everything it starts. The alias starts the package's
/// program, so with the same file name and version they are the same program.
fn same_store_app(a: &FingerprintEntry, b: &FingerprintEntry) -> bool {
    let parts = |p: &str| -> Vec<String> { p.split(['\\', '/']).map(str::to_ascii_lowercase).collect() };
    let is_alias = |p: &[String]| {
        let n = p.len();
        n >= 3 && p[n - 2] == "windowsapps" && p[n - 3] == "microsoft"
    };
    let in_package = |p: &[String]| p.len() >= 3 && p[..p.len() - 2].iter().any(|c| c == "windowsapps");
    let (Some(pa), Some(pb)) = (&a.path, &b.path) else { return false };
    let (pa, pb) = (parts(pa), parts(pb));
    a.version.is_some()
        && a.version == b.version
        && pa.last() == pb.last()
        && ((is_alias(&pa) && in_package(&pb)) || (is_alias(&pb) && in_package(&pa)))
}

/// The steps a set of drifts puts in question. A change of operating system,
/// build or architecture affects every step; a shell affects the steps bound
/// to it; a tool affects the steps, and plans, that require it.
pub fn affected_by_drift(plan: &Plan, graph: &PlanGraph, drifts: &[Drift]) -> Vec<String> {
    let everything = drifts.iter().any(|d| matches!(d.what.as_str(), "os" | "build" | "architecture"))
        || drifts.iter().any(|d| {
            d.what
                .strip_prefix("tool:")
                .is_some_and(|t| plan.requirements.iter().any(|r| r.executable == t || r.name == t))
        });
    let touches = |id: &str| -> bool {
        if everything {
            return true;
        }
        let Some(step) = plan.step(id) else { return false };
        drifts.iter().any(|d| {
            if let Some(shell) = d.what.strip_prefix("shell:") {
                step.shell.as_ref().is_some_and(|s| shell_name(s.kind) == shell)
            } else if let Some(tool) = d.what.strip_prefix("tool:") {
                step.tool_requirements.iter().any(|r| r.executable == tool || r.name == tool)
            } else {
                false
            }
        })
    };
    let mut affected = std::collections::BTreeSet::new();
    for id in graph.topological_order() {
        if touches(id) {
            affected.insert(id.to_owned());
            affected.extend(graph.descendants(id));
        }
    }
    graph.topological_order().filter(|id| affected.contains(*id)).map(str::to_owned).collect()
}

/// The name a fingerprint uses for a shell, matching `ShellName`'s serialised form.
pub fn shell_name(kind: crate::model::ShellName) -> &'static str {
    match kind {
        crate::model::ShellName::Pwsh => "pwsh",
        crate::model::ShellName::WindowsPowershell => "windows_powershell",
        crate::model::ShellName::Cmd => "cmd",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_standard_test_vector() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    fn fp() -> EnvironmentFingerprint {
        EnvironmentFingerprint {
            os: "Windows 11 Pro 25H2".into(),
            build: "26200.9457".into(),
            architecture: "x64".into(),
            shells: vec![FingerprintEntry {
                name: "pwsh".into(),
                path: Some("C:/pwsh.exe".into()),
                version: Some("7.6.6".into()),
            }],
            tools: vec![FingerprintEntry {
                name: "docker".into(),
                path: Some("C:/docker.exe".into()),
                version: None,
            }],
        }
    }

    #[test]
    fn a_fingerprint_hash_ignores_list_order_but_not_content() {
        let mut a = fp();
        a.tools.push(FingerprintEntry { name: "git".into(), path: None, version: None });
        let mut b = a.clone();
        b.tools.reverse();
        assert_eq!(a.hash(), b.hash());
        b.tools[0].path = Some("C:/elsewhere/git.exe".into());
        assert_ne!(a.hash(), b.hash());
    }

    /// A Store app is found through its alias from a process started from the
    /// Start menu, and through its package folder from one started by
    /// PowerShell 7, which puts its own folder first on PATH. Approved in the
    /// app and used from the CLI, the same pwsh.exe looked like a change.
    #[test]
    fn a_store_apps_alias_and_its_package_are_the_same_program() {
        let alias = r"C:\Users\nrtat\AppData\Local\Microsoft\WindowsApps\pwsh.exe";
        let package =
            r"C:\Program Files\WindowsApps\Microsoft.PowerShell_7.6.6.0_x64__8wekyb3d8bbwe\pwsh.exe";
        let with = |path: &str, version: &str| {
            let mut f = fp();
            f.tools = vec![FingerprintEntry {
                name: "pwsh.exe".into(),
                path: Some(path.into()),
                version: Some(version.into()),
            }];
            f
        };
        assert!(with(alias, "7.6.6").drift(&with(package, "7.6.6")).is_empty());
        assert!(with(package, "7.6.6").drift(&with(alias, "7.6.6")).is_empty());
        // Still a change: another version, another program, or two ordinary folders.
        assert_eq!(with(alias, "7.6.6").drift(&with(package, "7.7.0")).len(), 1);
        let other = r"C:\Program Files\WindowsApps\Other_1.0_x64__x\other.exe";
        assert_eq!(with(alias, "7.6.6").drift(&with(other, "7.6.6")).len(), 1);
        assert_eq!(with(r"C:\a\pwsh.exe", "7.6.6").drift(&with(r"C:\b\pwsh.exe", "7.6.6")).len(), 1);
    }

    #[test]
    fn drift_names_what_changed() {
        let before = fp();
        let mut after = fp();
        after.shells[0].version = Some("7.7.0".into());
        after.tools[0].path = None;
        let d = before.drift(&after);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].what, "shell:pwsh");
        assert_eq!(d[0].after.as_deref(), Some("7.7.0 at C:/pwsh.exe"));
        assert_eq!(d[1].what, "tool:docker");
        assert_eq!(d[1].after.as_deref(), Some("not found"));
        assert!(before.drift(&before).is_empty());
    }
}
