//! Work profiles: stricter safety for a kind of work.
//!
//! A profile says what kind of work a plan is for, such as IT Operations or
//! Helpdesk, and tightens what KeyJutsu demands of it. It adds instructions
//! to what the planning and reviewing agents are told, and it changes
//! validation: it can raise the risk of named commands, require a recovery
//! from a given risk upwards, cap the risk a step may have, refuse code
//! KeyJutsu cannot see into, and keep a deny list that fails validation.
//!
//! Profiles are TOML. Six are built into KeyJutsu. Every other profile
//! extends exactly one other, and each link in that chain can only tighten:
//! a profile that would loosen what it extends is refused, never quietly
//! clamped, so a profile always means at least what its parent means.
//! Deny and raise rules only add up; nothing can take one away.
//!
//! Commands are matched by name as PowerShell resolved them (an alias by what
//! it stands for, a program by its file name), never by searching the raw
//! text, and code a line runs inside another shell (`pwsh -Command`,
//! `-EncodedCommand`, `cmd /c`, `Invoke-Expression`) is unpacked and matched
//! too (see [`crate::nested`]).

use std::collections::BTreeMap;
use std::path::PathBuf;

use keyjutsu_plan::model::{Readiness, RecoveryStrategy, RiskLevel, Step, StepKind};
use serde::Deserialize;

use crate::risk::Assessment;

/// The built-in profiles, from least to most strict along each chain.
const BUILT_IN: &[(&str, &str)] = &[
    ("developer", include_str!("../profiles/developer.toml")),
    ("packaging", include_str!("../profiles/packaging.toml")),
    ("helpdesk", include_str!("../profiles/helpdesk.toml")),
    ("it-operations", include_str!("../profiles/it-operations.toml")),
    ("security-response", include_str!("../profiles/security-response.toml")),
    ("audit-only", include_str!("../profiles/audit-only.toml")),
];

/// The longest chain of `extends` KeyJutsu follows. Six built-ins and a few
/// layers of house rules fit easily; anything longer is a mistake.
pub const MAX_DEPTH: usize = 12;

/// Where a profile was defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    BuiltIn,
    /// A file in the machine's profiles folder.
    Machine(PathBuf),
    /// A file in a folder's `.keyjutsu` directory.
    Folder(PathBuf),
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::BuiltIn => f.write_str("built into KeyJutsu"),
            Origin::Machine(p) => write!(f, "this machine's {}", p.display()),
            Origin::Folder(p) => write!(f, "the folder's {}", p.display()),
        }
    }
}

/// A profile as written, before its chain is resolved.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFile {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub summary: Option<String>,
    /// The one profile this one tightens. Only the root built-in has none.
    #[serde(default)]
    pub extends: Option<String>,
    /// Added to the ground rules every planning and reviewing agent is given.
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub validation: ValidationRules,
    #[serde(default)]
    pub approval: ApprovalRules,
}

/// What the operator must do before a step runs.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRules {
    /// A step at this risk or above needs its own typed confirmation, as a
    /// critical step always does.
    #[serde(default)]
    pub confirm_each_from: Option<RiskLevel>,
    /// Reserved for two-person approval, which KeyJutsu does not have yet.
    /// Any value is refused rather than ignored, so nobody believes a second
    /// person is being asked when nobody is.
    #[serde(default)]
    pub second_person: Option<toml::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationRules {
    /// A step at this risk or above must say how to recover from it.
    #[serde(default)]
    pub require_recovery_from: Option<RiskLevel>,
    /// The highest risk a step may have; anything above fails validation.
    #[serde(default)]
    pub max_risk: Option<RiskLevel>,
    /// Refuse code KeyJutsu cannot see into: a command whose name is
    /// computed when it runs, a script file handed to a shell, or nested code
    /// that is not written out literally.
    #[serde(default)]
    pub block_opaque_code: Option<bool>,
    #[serde(default)]
    pub raise: Vec<RaiseRule>,
    #[serde(default)]
    pub deny: Vec<DenyRule>,
}

/// Commands whose risk is at least `to` under this profile.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RaiseRule {
    pub commands: Vec<String>,
    pub to: RiskLevel,
    pub reason: String,
}

/// Commands this profile refuses.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DenyRule {
    pub commands: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("{origin}: {message}")]
    Parse { origin: String, message: String },
    #[error("{origin}: \"{name}\" is not a profile name; use 1 to 64 lower-case letters, digits and hyphens")]
    BadName { origin: String, name: String },
    #[error("the profile \"{name}\" is defined twice: in {first} and in {second}")]
    Clash { name: String, first: String, second: String },
    #[error("there is no profile called \"{0}\"")]
    Unknown(String),
    #[error("the profile \"{name}\" extends \"{extends}\", which is not defined")]
    MissingParent { name: String, extends: String },
    #[error(
        "the profile \"{0}\" must extend another profile; only KeyJutsu's own root profile extends nothing"
    )]
    NoParent(String),
    #[error("profiles extend each other in a circle: {0}")]
    Cycle(String),
    #[error("the profile \"{name}\" is more than {MAX_DEPTH} profiles deep")]
    TooDeep { name: String },
    #[error("the profile \"{name}\" loosens what \"{parent}\" sets: {what}")]
    Loosens { name: String, parent: String, what: String },
    #[error("the profile \"{name}\" has an empty pattern in {rule}")]
    EmptyPattern { name: String, rule: String },
    #[error("the profile \"{0}\" sets approval.second_person, but KeyJutsu cannot ask a second person yet")]
    SecondPerson(String),
}

/// Every profile KeyJutsu knows about, wherever it was defined.
#[derive(Debug, Clone)]
pub struct Catalogue {
    entries: BTreeMap<String, (Origin, ProfileFile)>,
}

impl Catalogue {
    /// The built-in profiles alone.
    pub fn built_in() -> Self {
        let mut c = Self { entries: BTreeMap::new() };
        for (_, text) in BUILT_IN {
            // The built-ins are tested to parse; a failure here is a build fault.
            #[allow(clippy::expect_used)]
            c.add(Origin::BuiltIn, text).expect("a built-in profile parses");
        }
        c
    }

    /// Add a profile defined in TOML. A name already defined anywhere is a
    /// clash: nobody should run under a profile that silently differs from
    /// the one they read.
    pub fn add(&mut self, origin: Origin, text: &str) -> Result<(), ProfileError> {
        let file: ProfileFile = toml::from_str(text).map_err(|e| ProfileError::Parse {
            origin: origin.to_string(),
            message: e.message().to_owned(),
        })?;
        if !valid_name(&file.name) {
            return Err(ProfileError::BadName { origin: origin.to_string(), name: file.name });
        }
        if let Some((first, _)) = self.entries.get(&file.name) {
            return Err(ProfileError::Clash {
                name: file.name.clone(),
                first: first.to_string(),
                second: origin.to_string(),
            });
        }
        if file.extends.is_none() && origin != Origin::BuiltIn {
            return Err(ProfileError::NoParent(file.name));
        }
        self.entries.insert(file.name.clone(), (origin, file));
        Ok(())
    }

    /// Profile names with their titles, built-ins first in their own order.
    pub fn names(&self) -> Vec<(String, String)> {
        let built: Vec<&str> = BUILT_IN.iter().map(|(n, _)| *n).collect();
        let mut out: Vec<(String, String)> = built
            .iter()
            .filter_map(|n| self.entries.get(*n).map(|(_, f)| ((*n).to_owned(), f.title.clone())))
            .collect();
        out.extend(
            self.entries
                .iter()
                .filter(|(n, _)| !built.contains(&n.as_str()))
                .map(|(n, (_, f))| (n.clone(), f.title.clone())),
        );
        out
    }

    /// The profile `name` with its whole chain applied.
    pub fn resolve(&self, name: &str) -> Result<Profile, ProfileError> {
        // Walk up to the root, then apply from the root down.
        let mut chain: Vec<&str> = Vec::new();
        let mut at = name;
        loop {
            if let Some(i) = chain.iter().position(|n| *n == at) {
                let mut circle: Vec<&str> = chain[i..].to_vec();
                circle.push(at);
                return Err(ProfileError::Cycle(circle.join(" â†’ ")));
            }
            if chain.len() >= MAX_DEPTH {
                return Err(ProfileError::TooDeep { name: name.to_owned() });
            }
            let Some((_, file)) = self.entries.get(at) else {
                return Err(match chain.last() {
                    None => ProfileError::Unknown(at.to_owned()),
                    Some(child) => {
                        ProfileError::MissingParent { name: (*child).to_owned(), extends: at.to_owned() }
                    }
                });
            };
            chain.push(at);
            match &file.extends {
                Some(parent) => at = parent,
                None => break,
            }
        }
        let mut profile: Option<Profile> = None;
        for link in chain.iter().rev() {
            let (origin, file) = &self.entries[*link];
            profile = Some(match profile {
                None => Profile::root(origin, file)?,
                Some(parent) => parent.tighten(origin, file)?,
            });
        }
        // The loop ran at least once: `chain` holds `name` itself.
        profile.ok_or_else(|| ProfileError::Unknown(name.to_owned()))
    }
}

fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// One link in a resolved profile's chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub name: String,
    pub title: String,
    pub summary: Option<String>,
    pub origin: Origin,
}

/// A rule with the profile it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Sourced<T> {
    pub profile: String,
    pub rule: T,
}

/// A profile with everything it inherits applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// From the profile itself up to the root.
    pub chain: Vec<Link>,
    /// Each profile's instructions, root first.
    pub instructions: Vec<Sourced<String>>,
    pub require_recovery_from: Option<RiskLevel>,
    pub max_risk: Option<RiskLevel>,
    pub block_opaque_code: bool,
    pub raise: Vec<Sourced<RaiseRule>>,
    pub deny: Vec<Sourced<DenyRule>>,
    /// From this risk, a step needs its own typed confirmation; only
    /// critical steps otherwise.
    pub confirm_each_from: Option<RiskLevel>,
}

impl Profile {
    pub fn name(&self) -> &str {
        &self.chain[0].name
    }

    pub fn title(&self) -> &str {
        &self.chain[0].title
    }

    /// Whether `other` is this profile or one it extends, so that this
    /// profile asks at least as much as `other` does.
    pub fn at_least_as_strict_as(&self, other: &str) -> bool {
        self.chain.iter().any(|l| l.name == other)
    }

    fn root(origin: &Origin, file: &ProfileFile) -> Result<Self, ProfileError> {
        let empty = Self {
            chain: Vec::new(),
            instructions: Vec::new(),
            require_recovery_from: None,
            max_risk: None,
            block_opaque_code: false,
            raise: Vec::new(),
            deny: Vec::new(),
            confirm_each_from: None,
        };
        empty.tighten(origin, file)
    }

    /// This profile with `file`, which extends it, applied on top.
    fn tighten(mut self, origin: &Origin, file: &ProfileFile) -> Result<Self, ProfileError> {
        let parent = self.chain.first().map(|l| l.name.clone()).unwrap_or_default();
        let loosens =
            |what: String| ProfileError::Loosens { name: file.name.clone(), parent: parent.clone(), what };
        let v = &file.validation;
        if let Some(from) = v.require_recovery_from {
            match self.require_recovery_from {
                Some(was) if from > was => {
                    return Err(loosens(format!(
                        "it requires a recovery only from {} risk, where \"{parent}\" requires one from {}",
                        level(from),
                        level(was)
                    )));
                }
                _ => self.require_recovery_from = Some(from),
            }
        }
        if let Some(max) = v.max_risk {
            match self.max_risk {
                Some(was) if max > was => {
                    return Err(loosens(format!(
                        "it allows {} risk, where \"{parent}\" allows nothing above {}",
                        level(max),
                        level(was)
                    )));
                }
                _ => self.max_risk = Some(max),
            }
        }
        match v.block_opaque_code {
            Some(false) if self.block_opaque_code => {
                return Err(loosens(format!(
                    "it allows code KeyJutsu cannot see into, which \"{parent}\" refuses"
                )));
            }
            Some(b) => self.block_opaque_code = b,
            None => {}
        }
        if file.approval.second_person.is_some() {
            return Err(ProfileError::SecondPerson(file.name.clone()));
        }
        if let Some(from) = file.approval.confirm_each_from {
            match self.confirm_each_from {
                Some(was) if from > was => {
                    return Err(loosens(format!(
                        "it asks for each step's own confirmation only from {} risk, where \"{parent}\" asks from {}",
                        level(from),
                        level(was)
                    )));
                }
                _ => self.confirm_each_from = Some(from),
            }
        }
        for (i, r) in v.raise.iter().enumerate() {
            if r.commands.is_empty() || r.commands.iter().any(|c| c.trim().is_empty()) {
                return Err(ProfileError::EmptyPattern {
                    name: file.name.clone(),
                    rule: format!("raise rule {}", i + 1),
                });
            }
            self.raise.push(Sourced { profile: file.name.clone(), rule: r.clone() });
        }
        for (i, r) in v.deny.iter().enumerate() {
            if r.commands.is_empty() || r.commands.iter().any(|c| c.trim().is_empty()) {
                return Err(ProfileError::EmptyPattern {
                    name: file.name.clone(),
                    rule: format!("deny rule {}", i + 1),
                });
            }
            self.deny.push(Sourced { profile: file.name.clone(), rule: r.clone() });
        }
        if let Some(text) = file.instructions.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            self.instructions.push(Sourced { profile: file.name.clone(), rule: text.to_owned() });
        }
        self.chain.insert(
            0,
            Link {
                name: file.name.clone(),
                title: file.title.clone(),
                summary: file.summary.clone(),
                origin: origin.clone(),
            },
        );
        Ok(self)
    }

    /// What the agents are told, added to their ground rules.
    pub fn agent_rules(&self) -> String {
        let mut s = format!(
            "This plan is for {} work (KeyJutsu work profile \"{}\"). Its rules come on top of the ones above, \
             and KeyJutsu's validation enforces them:\n",
            self.title(),
            self.name()
        );
        for i in &self.instructions {
            s.push_str(&format!("- {}\n", i.rule.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
        if let Some(r) = self.require_recovery_from {
            s.push_str(&format!(
                "- Every step rated {} risk or above needs a recovery (strategy restore_captured_state or commands).\n",
                level(r)
            ));
        }
        if let Some(m) = self.max_risk {
            s.push_str(&format!("- No step may be rated above {} risk.\n", level(m)));
        }
        if self.block_opaque_code {
            s.push_str(
                "- Write every command out literally: no -EncodedCommand, no Invoke-Expression, no command names \
                 held in variables, no script files handed to a shell.\n",
            );
        }
        if !self.deny.is_empty() {
            let names: Vec<&str> =
                self.deny.iter().flat_map(|d| d.rule.commands.iter().map(String::as_str)).collect();
            s.push_str(&format!("- These are refused, whatever the task: {}.\n", names.join(", ")));
        }
        s.trim_end().to_owned()
    }
}

/// One command a step would run, as a profile matches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    /// Its identity (see [`identity`]); for an alias, what it stands for.
    pub name: String,
    /// The alias it was written as, if it was one.
    pub written_as: Option<String>,
    pub arguments: Vec<String>,
    /// How it is reached when it is nested, such as "inside `cmd /c`".
    pub via: Option<String>,
}

impl Use {
    fn matches(&self, pattern: &str) -> bool {
        matches(pattern, &self.name, &self.arguments)
            || self.written_as.as_deref().is_some_and(|w| matches(pattern, w, &self.arguments))
    }

    fn describe(&self) -> String {
        match &self.via {
            Some(via) => format!("`{}` {via}", self.name),
            None => format!("`{}`", self.name),
        }
    }
}

/// Everything a profile looks at in one step: the commands it would run,
/// nested ones included, and the code it could not see into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seen {
    pub uses: Vec<Use>,
    pub opaque: Vec<String>,
}

/// Where a step fails a profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breach {
    pub readiness: Readiness,
    pub detail: String,
}

impl Profile {
    /// Apply this profile to one step: raise its risk where a rule says so,
    /// then say everywhere it falls short. `risk` must already hold
    /// KeyJutsu's own assessment, the agent's rating included, since the
    /// recovery and ceiling rules turn on the final level.
    pub fn apply(&self, step: &Step, seen: &Seen, risk: &mut Assessment) -> Vec<Breach> {
        let title = self.title();
        for r in &self.raise {
            for u in &seen.uses {
                if r.rule.commands.iter().any(|p| u.matches(p)) {
                    risk.raise(r.rule.to, format!("{title}: {} {}", u.describe(), r.rule.reason));
                }
            }
        }
        risk.reasons.dedup();
        let mut out = Vec::new();
        for d in &self.deny {
            for u in &seen.uses {
                if let Some(p) = d.rule.commands.iter().find(|p| u.matches(p)) {
                    out.push(Breach {
                        readiness: Readiness::Blocked,
                        detail: format!(
                            "{} is refused under {title} (`{p}`, from \"{}\"): it {}",
                            u.describe(),
                            d.profile,
                            d.rule.reason
                        ),
                    });
                }
            }
        }
        if self.block_opaque_code {
            for o in &seen.opaque {
                out.push(Breach {
                    readiness: Readiness::Blocked,
                    detail: format!("{title} refuses code KeyJutsu cannot see into: {o}"),
                });
            }
        }
        if let Some(max) = self.max_risk
            && risk.level > max
        {
            out.push(Breach {
                readiness: Readiness::Blocked,
                detail: format!(
                    "{title} allows nothing above {} risk, and this step is {}: {}",
                    level(max),
                    level(risk.level),
                    risk.reasons.join("; ")
                ),
            });
        }
        let operator_step =
            matches!(step.kind, StepKind::Manual | StepKind::UserInput | StepKind::Credential);
        let recovers = step.recovery.as_ref().is_some_and(|r| r.strategy != RecoveryStrategy::None);
        if let Some(from) = self.require_recovery_from
            && risk.level >= from
            && !operator_step
            && !recovers
        {
            out.push(Breach {
                readiness: Readiness::Invalid,
                detail: format!(
                    "{title} needs a recovery for every step at {} risk or above, and this {} step has none",
                    level(from),
                    level(risk.level)
                ),
            });
        }
        out.dedup();
        out
    }
}

pub(crate) fn level(r: RiskLevel) -> &'static str {
    match r {
        RiskLevel::Low => "low",
        RiskLevel::Normal => "normal",
        RiskLevel::High => "high",
        RiskLevel::Critical => "critical",
    }
}

/// Whether a command matches a pattern from a profile.
///
/// A pattern is a command name, with `*` and `?` as wildcards and any case,
/// optionally followed by the start of its arguments: `Remove-AD*` matches
/// every Active Directory removal, `wevtutil cl*` only clearing a log.
/// `name` is the command as PowerShell resolved it, without a module
/// qualifier, a folder or an extension such as `.exe`.
pub fn matches(pattern: &str, name: &str, arguments: &[String]) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let (p_name, p_args) = match pattern.split_once(char::is_whitespace) {
        Some((n, a)) => (n, Some(a.trim())),
        None => (pattern.as_str(), None),
    };
    if !glob(p_name, &name.to_ascii_lowercase()) {
        return false;
    }
    match p_args {
        None => true,
        Some(p) => {
            let args =
                arguments.iter().map(|a| unquote(a)).collect::<Vec<_>>().join(" ").to_ascii_lowercase();
            // The start of the arguments: a pattern names what matters first.
            glob(&format!("{p}*"), &args)
        }
    }
}

/// The identity a pattern is matched against: no module qualifier, folder or
/// program extension, so `Microsoft.PowerShell.Management\Remove-Item`,
/// `C:\Windows\System32\format.com` and `FORMAT` are each one name.
pub fn identity(name: &str) -> String {
    let base = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let lower = base.to_ascii_lowercase();
    for ext in [".exe", ".com", ".cmd", ".bat"] {
        if let Some(stem) = lower.strip_suffix(ext) {
            return stem.to_owned();
        }
    }
    lower
}

/// A literal argument without its quotes.
pub(crate) fn unquote(arg: &str) -> String {
    let a = arg.trim();
    for q in ['\'', '"'] {
        if a.len() >= 2 && a.starts_with(q) && a.ends_with(q) {
            return a[1..a.len() - 1].to_owned();
        }
    }
    a.to_owned()
}

/// `*` for any run of characters, `?` for one.
fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(extra: &[&str]) -> Catalogue {
        let mut c = Catalogue::built_in();
        for (i, t) in extra.iter().enumerate() {
            c.add(Origin::Machine(PathBuf::from(format!("p{i}.toml"))), t).unwrap();
        }
        c
    }

    #[test]
    fn every_built_in_resolves() {
        let c = Catalogue::built_in();
        for (name, _) in BUILT_IN {
            let p = c.resolve(name).unwrap();
            assert_eq!(p.name(), *name);
            assert!(!p.agent_rules().is_empty());
        }
    }

    #[test]
    fn the_built_ins_tighten_down_the_chain() {
        let c = Catalogue::built_in();
        let audit = c.resolve("audit-only").unwrap();
        for name in ["developer", "helpdesk", "it-operations", "security-response"] {
            assert!(audit.at_least_as_strict_as(name), "audit-only should be at least {name}");
        }
        assert_eq!(audit.max_risk, Some(RiskLevel::Low));
        let dev = c.resolve("developer").unwrap();
        assert!(!dev.at_least_as_strict_as("it-operations"));
        assert!(c.resolve("it-operations").unwrap().block_opaque_code);
    }

    #[test]
    fn a_profile_may_extend_one_from_another_place() {
        let c = with(&[
            "name = \"client-x\"\ntitle = \"Client X\"\nextends = \"it-operations\"\n\
             [[validation.deny]]\ncommands = [\"Set-Mailbox\"]\nreason = \"mail is the client's own team's\"",
            "name = \"client-x-project\"\ntitle = \"Client X project\"\nextends = \"client-x\"",
        ]);
        let p = c.resolve("client-x-project").unwrap();
        let names: Vec<&str> = p.chain.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["client-x-project", "client-x", "it-operations", "helpdesk", "developer"]);
        assert!(p.deny.iter().any(|d| d.profile == "client-x"));
        assert!(p.at_least_as_strict_as("helpdesk"));
    }

    #[test]
    fn a_profile_cannot_loosen_what_it_extends() {
        let c = with(&[
            "name = \"lax\"\ntitle = \"Lax\"\nextends = \"it-operations\"\n[validation]\nrequire_recovery_from = \"critical\"",
            "name = \"see-nothing\"\ntitle = \"S\"\nextends = \"it-operations\"\n[validation]\nblock_opaque_code = false",
            "name = \"wider\"\ntitle = \"W\"\nextends = \"audit-only\"\n[validation]\nmax_risk = \"normal\"",
        ]);
        for name in ["lax", "see-nothing", "wider"] {
            assert!(
                matches!(c.resolve(name), Err(ProfileError::Loosens { .. })),
                "{name}: {:?}",
                c.resolve(name)
            );
        }
        // Tightening further is fine.
        let c = with(&[
            "name = \"tight\"\ntitle = \"T\"\nextends = \"it-operations\"\n[validation]\nrequire_recovery_from = \"low\"",
        ]);
        assert_eq!(c.resolve("tight").unwrap().require_recovery_from, Some(RiskLevel::Low));
    }

    #[test]
    fn a_resolved_chain_is_never_looser_than_any_profile_in_it() {
        let c = Catalogue::built_in();
        for (name, _) in BUILT_IN {
            let p = c.resolve(name).unwrap();
            for link in &p.chain[1..] {
                let ancestor = c.resolve(&link.name).unwrap();
                if let Some(a) = ancestor.require_recovery_from {
                    assert!(p.require_recovery_from.is_some_and(|r| r <= a), "{name} vs {}", link.name);
                }
                if let Some(a) = ancestor.max_risk {
                    assert!(p.max_risk.is_some_and(|m| m <= a), "{name} vs {}", link.name);
                }
                assert!(p.block_opaque_code || !ancestor.block_opaque_code);
                assert!(ancestor.deny.iter().all(|d| p.deny.contains(d)));
                assert!(ancestor.raise.iter().all(|r| p.raise.contains(r)));
            }
        }
    }

    #[test]
    fn cycles_depth_clashes_and_orphans_are_refused() {
        let c = with(&[
            "name = \"a\"\ntitle = \"A\"\nextends = \"b\"",
            "name = \"b\"\ntitle = \"B\"\nextends = \"a\"",
            "name = \"orphan\"\ntitle = \"O\"\nextends = \"nowhere\"",
        ]);
        assert_eq!(c.resolve("a"), Err(ProfileError::Cycle("a â†’ b â†’ a".into())));
        assert!(matches!(c.resolve("orphan"), Err(ProfileError::MissingParent { .. })));
        assert!(matches!(c.resolve("absent"), Err(ProfileError::Unknown(_))));

        let mut deep = Catalogue::built_in();
        let mut parent = "developer".to_owned();
        for i in 0..MAX_DEPTH {
            let name = format!("d{i}");
            deep.add(
                Origin::Machine("x".into()),
                &format!("name = \"{name}\"\ntitle = \"D\"\nextends = \"{parent}\""),
            )
            .unwrap();
            parent = name;
        }
        assert!(matches!(deep.resolve(&parent), Err(ProfileError::TooDeep { .. })));

        let mut clash = Catalogue::built_in();
        let err = clash
            .add(Origin::Folder("f".into()), "name = \"helpdesk\"\ntitle = \"Mine\"\nextends = \"developer\"")
            .unwrap_err();
        assert!(matches!(err, ProfileError::Clash { .. }), "{err}");
        let err = clash.add(Origin::Folder("f".into()), "name = \"root\"\ntitle = \"R\"").unwrap_err();
        assert_eq!(err, ProfileError::NoParent("root".into()));
    }

    #[test]
    fn unknown_keys_and_bad_names_are_refused() {
        let mut c = Catalogue::built_in();
        let err = c
            .add(
                Origin::Machine("m".into()),
                "name = \"x\"\ntitle = \"X\"\nextends = \"developer\"\nstrict = true",
            )
            .unwrap_err();
        assert!(matches!(err, ProfileError::Parse { .. }), "{err}");
        let err = c
            .add(Origin::Machine("m".into()), "name = \"Has Space\"\ntitle = \"X\"\nextends = \"developer\"");
        assert!(matches!(err, Err(ProfileError::BadName { .. })));
        let err = c.add(Origin::Machine("m".into()), "name = \"e\"\ntitle = \"E\"\nextends = \"developer\"\n[[validation.deny]]\ncommands = [\" \"]\nreason = \"r\"");
        assert!(err.is_ok());
        assert!(matches!(c.resolve("e"), Err(ProfileError::EmptyPattern { .. })));
    }

    #[test]
    fn approval_settings_only_tighten_and_a_second_person_is_refused() {
        let c = Catalogue::built_in();
        assert_eq!(c.resolve("it-operations").unwrap().confirm_each_from, Some(RiskLevel::High));
        assert_eq!(c.resolve("audit-only").unwrap().confirm_each_from, Some(RiskLevel::High));
        assert_eq!(c.resolve("helpdesk").unwrap().confirm_each_from, None);
        let c = with(&[
            "name = \"looser\"
title = \"L\"
extends = \"it-operations\"
[approval]
confirm_each_from = \"critical\"",
            "name = \"tighter\"
title = \"T\"
extends = \"it-operations\"
[approval]
confirm_each_from = \"normal\"",
            "name = \"pair\"
title = \"P\"
extends = \"developer\"
[approval.second_person]
from = \"high\"",
        ]);
        assert!(
            matches!(c.resolve("looser"), Err(ProfileError::Loosens { .. })),
            "{:?}",
            c.resolve("looser")
        );
        assert_eq!(c.resolve("tighter").unwrap().confirm_each_from, Some(RiskLevel::Normal));
        assert_eq!(c.resolve("pair"), Err(ProfileError::SecondPerson("pair".into())));
    }

    #[test]
    fn patterns_match_names_and_the_start_of_arguments() {
        let none: [String; 0] = [];
        assert!(matches("Remove-AD*", "remove-aduser", &none));
        assert!(matches("format", "format", &["c:".into()]));
        assert!(!matches("format", "format-table", &none));
        assert!(matches("wevtutil cl*", "wevtutil", &["cl".into(), "Security".into()]));
        assert!(matches("wevtutil cl*", "wevtutil", &["'clear-log'".into()]));
        assert!(!matches("wevtutil cl*", "wevtutil", &["qe".into(), "Security".into()]));
        assert!(matches("git push", "git", &["push".into(), "--force".into()]));
        assert!(matches("*-mg*", "remove-mguser", &none));
        assert_eq!(identity("C:\\Windows\\System32\\FORMAT.COM"), "format");
        assert_eq!(identity("Microsoft.PowerShell.Management\\Remove-Item"), "remove-item");
    }
}
