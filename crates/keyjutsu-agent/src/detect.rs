//! Finding the installed agents.
//!
//! Detection runs each agent CLI once with `--version`. These are programs the
//! operator installed to run, not programs a plan names, so asking them their
//! version is ordinary use. Whether an agent is signed in is judged only from
//! whether its credential file exists: the file is never opened, because its
//! contents are a secret.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use keyjutsu_validation::probe::resolve_executable;
use keyjutsu_validation::process;
use serde::Serialize;

use crate::agents::{AgentKind, Capabilities};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "agent/")]
pub enum SignIn {
    /// A credential file or key variable exists. It may still have expired.
    CredentialsFound,
    NoCredentialsFound,
    /// This agent keeps no credential file KeyJutsu knows how to look for.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "agent/")]
pub struct AgentInfo {
    pub kind: AgentKind,
    pub name: &'static str,
    pub path: Option<String>,
    pub version: Option<String>,
    pub sign_in: SignIn,
    pub capabilities: Capabilities,
    /// The installed version differs from the one the adapter was checked
    /// against, so its flags should be re-checked.
    pub needs_compatibility_check: bool,
}

impl AgentInfo {
    pub fn installed(&self) -> bool {
        self.path.is_some()
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

/// Existence only: see the module comment.
fn sign_in(kind: AgentKind) -> SignIn {
    let exists = |rel: &str| home().is_some_and(|h| h.join(rel).exists());
    match kind {
        AgentKind::Codex => {
            if exists(".codex/auth.json") || std::env::var_os("OPENAI_API_KEY").is_some() {
                SignIn::CredentialsFound
            } else {
                SignIn::NoCredentialsFound
            }
        }
        AgentKind::ClaudeCode => {
            if exists(".claude/.credentials.json") || std::env::var_os("ANTHROPIC_API_KEY").is_some() {
                SignIn::CredentialsFound
            } else {
                SignIn::NoCredentialsFound
            }
        }
        AgentKind::Gemini => {
            if exists(".gemini/oauth_creds.json")
                || std::env::var_os("GEMINI_API_KEY").is_some()
                || std::env::var_os("GOOGLE_API_KEY").is_some()
            {
                SignIn::CredentialsFound
            } else {
                SignIn::NoCredentialsFound
            }
        }
        AgentKind::GithubCopilot | AgentKind::Cursor => SignIn::Unknown,
    }
}

/// The first version-looking token in `text`: `codex-cli 0.154.0` gives
/// `0.154.0`, `GitHub Copilot CLI 1.0.78.` gives `1.0.78`.
pub fn version_in(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .map(|t| t.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .map(|t| t.trim_start_matches(['v', 'V']))
        .find(|t| {
            t.contains('.')
                && t.split('.').next().is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        })
        .map(str::to_owned)
}

fn version_of(path: &Path) -> Option<String> {
    let mut c = Command::new(path);
    c.arg("--version");
    let done = process::run(c, "", Duration::from_secs(20)).ok()?;
    version_in(&done.stdout).or_else(|| version_in(&done.stderr))
}

pub fn detect(kind: AgentKind) -> AgentInfo {
    let path = resolve_executable(kind.executable());
    let version = path.as_deref().and_then(version_of);
    let capabilities = kind.capabilities();
    let needs_compatibility_check = path.is_some()
        && (capabilities.verified_with.is_none() || capabilities.verified_with != version.as_deref());
    AgentInfo {
        kind,
        name: kind.display_name(),
        path: path.map(|p| p.display().to_string()),
        version,
        sign_in: sign_in(kind),
        capabilities,
        needs_compatibility_check,
    }
}

/// Every supported agent, installed or not, detected in parallel.
pub fn detect_all() -> Vec<AgentInfo> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = AgentKind::ALL.into_iter().map(|k| scope.spawn(move || detect(k))).collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_versions_these_clis_actually_print() {
        assert_eq!(version_in("codex-cli 0.154.0").as_deref(), Some("0.154.0"));
        assert_eq!(version_in("2.1.282 (Claude Code)").as_deref(), Some("2.1.282"));
        assert_eq!(version_in("0.32.1\n").as_deref(), Some("0.32.1"));
        assert_eq!(
            version_in("GitHub Copilot CLI 1.0.78.\nRun 'copilot update' to check for updates.").as_deref(),
            Some("1.0.78")
        );
        assert_eq!(version_in("no version here"), None);
    }

    #[test]
    fn an_agent_that_is_not_installed_is_reported_as_such() {
        let info = AgentInfo {
            kind: AgentKind::Cursor,
            name: "Cursor CLI",
            path: None,
            version: None,
            sign_in: SignIn::Unknown,
            capabilities: AgentKind::Cursor.capabilities(),
            needs_compatibility_check: false,
        };
        assert!(!info.installed());
    }
}
