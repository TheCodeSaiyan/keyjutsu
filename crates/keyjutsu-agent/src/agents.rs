//! The five supported agents, how each is invoked non-interactively, and how
//! each is kept to investigating rather than changing anything.
//!
//! Every flag here was read from the agent's own `--help` on a real machine;
//! `verified_with` records which version. Agent CLIs change between versions
//! (§5), so detection compares the installed version with that and says when
//! they differ.

use std::path::{Path, PathBuf};

use keyjutsu_plan::model::AgentName;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "agent/")]
pub enum AgentKind {
    Codex,
    ClaudeCode,
    Gemini,
    GithubCopilot,
    Cursor,
}

impl AgentKind {
    pub const ALL: [AgentKind; 5] = [
        AgentKind::Codex,
        AgentKind::ClaudeCode,
        AgentKind::Gemini,
        AgentKind::GithubCopilot,
        AgentKind::Cursor,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            AgentKind::Codex => "Codex CLI",
            AgentKind::ClaudeCode => "Claude Code",
            AgentKind::Gemini => "Gemini CLI",
            AgentKind::GithubCopilot => "GitHub Copilot CLI",
            AgentKind::Cursor => "Cursor CLI",
        }
    }

    /// The executable name looked up on `PATH`.
    pub fn executable(self) -> &'static str {
        match self {
            AgentKind::Codex => "codex",
            AgentKind::ClaudeCode => "claude",
            AgentKind::Gemini => "gemini",
            AgentKind::GithubCopilot => "copilot",
            AgentKind::Cursor => "cursor-agent",
        }
    }

    pub fn plan_name(self) -> AgentName {
        match self {
            AgentKind::Codex => AgentName::Codex,
            AgentKind::ClaudeCode => AgentName::ClaudeCode,
            AgentKind::Gemini => AgentName::Gemini,
            AgentKind::GithubCopilot => AgentName::GithubCopilot,
            AgentKind::Cursor => AgentName::Cursor,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "codex" => Some(AgentKind::Codex),
            "claude" | "claude_code" => Some(AgentKind::ClaudeCode),
            "gemini" => Some(AgentKind::Gemini),
            "copilot" | "github_copilot" => Some(AgentKind::GithubCopilot),
            "cursor" | "cursor_agent" => Some(AgentKind::Cursor),
            _ => None,
        }
    }

    pub fn capabilities(self) -> Capabilities {
        match self {
            AgentKind::Codex => Capabilities {
                headless: "codex exec, prompt on stdin",
                read_only: "--sandbox read-only: model-run shell commands may read but not write",
                output: "-o FILE: the final message, written to a file",
                verified_with: Some("0.154.0"),
                prompt_on_stdin: true,
            },
            AgentKind::ClaudeCode => Capabilities {
                headless: "claude -p, prompt on stdin",
                read_only: "--permission-mode plan: plan mode, no edits or commands",
                output: "--output-format json: a JSON envelope around the final message",
                verified_with: Some("2.1.282"),
                prompt_on_stdin: true,
            },
            AgentKind::Gemini => Capabilities {
                headless: "gemini -p, prompt on stdin",
                read_only: "--approval-mode plan: described by the CLI as read-only mode",
                output: "-o json: a JSON envelope around the final message",
                verified_with: Some("0.32.1"),
                prompt_on_stdin: true,
            },
            AgentKind::GithubCopilot => Capabilities {
                headless: "copilot -p PROMPT (the prompt is an argument, so it must stay short)",
                read_only: "--mode plan with --no-ask-user; tools that change files are not allowed",
                output: "-s: only the agent's response, as text",
                verified_with: Some("1.0.78"),
                prompt_on_stdin: false,
            },
            AgentKind::Cursor => Capabilities {
                headless: "cursor-agent -p, prompt as an argument",
                read_only: "not verified: the Cursor agent CLI was not installed where this adapter was written",
                output: "--output-format json (not verified)",
                verified_with: None,
                prompt_on_stdin: false,
            },
        }
    }
}

/// What an adapter relies on, for the compatibility matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "agent/")]
pub struct Capabilities {
    pub headless: &'static str,
    pub read_only: &'static str,
    pub output: &'static str,
    /// The version whose `--help` these flags were read from, if any.
    pub verified_with: Option<&'static str>,
    pub prompt_on_stdin: bool,
}

/// How to run an agent once. Nothing here decides what the prompt says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub stdin: String,
    pub cwd: PathBuf,
    /// Where the agent writes its final message, when it does so to a file.
    pub output_file: Option<PathBuf>,
}

/// The longest prompt passed as a command-line argument. Windows allows
/// 32,767 characters for a whole command line; this leaves room for the rest.
pub const MAX_ARGUMENT_PROMPT: usize = 24_000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvocationError {
    #[error(
        "{agent} takes its prompt on the command line, and this prompt is {len} characters (limit {MAX_ARGUMENT_PROMPT})"
    )]
    PromptTooLong { agent: &'static str, len: usize },
}

/// Build the invocation for `kind`, always in its read-only mode.
pub fn invocation(
    kind: AgentKind,
    program: &Path,
    prompt: &str,
    cwd: &Path,
    scratch: &Path,
) -> Result<Invocation, InvocationError> {
    let caps = kind.capabilities();
    if !caps.prompt_on_stdin && prompt.len() > MAX_ARGUMENT_PROMPT {
        return Err(InvocationError::PromptTooLong { agent: kind.display_name(), len: prompt.len() });
    }
    let s = |x: &str| x.to_owned();
    let (args, stdin, output_file) = match kind {
        AgentKind::Codex => {
            let out = scratch.join("codex-last-message.txt");
            let args = vec![
                s("exec"),
                s("--sandbox"),
                s("read-only"),
                s("--skip-git-repo-check"),
                s("--ephemeral"),
                s("--color"),
                s("never"),
                s("-C"),
                cwd.display().to_string(),
                s("-o"),
                out.display().to_string(),
                s("-"),
            ];
            (args, prompt.to_owned(), Some(out))
        }
        AgentKind::ClaudeCode => (
            vec![
                s("-p"),
                s("--permission-mode"),
                s("plan"),
                s("--output-format"),
                s("json"),
                s("--no-session-persistence"),
            ],
            prompt.to_owned(),
            None,
        ),
        AgentKind::Gemini => (
            vec![
                s("-p"),
                s("Follow the instructions given on standard input."),
                s("--approval-mode"),
                s("plan"),
                s("-o"),
                s("json"),
            ],
            prompt.to_owned(),
            None,
        ),
        AgentKind::GithubCopilot => (
            vec![
                s("-p"),
                prompt.to_owned(),
                s("--mode"),
                s("plan"),
                s("--no-ask-user"),
                s("-s"),
                s("--no-color"),
            ],
            String::new(),
            None,
        ),
        AgentKind::Cursor => {
            (vec![s("-p"), prompt.to_owned(), s("--output-format"), s("json")], String::new(), None)
        }
    };
    Ok(Invocation { program: program.to_path_buf(), args, stdin, cwd: cwd.to_path_buf(), output_file })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(kind: AgentKind, prompt: &str) -> Invocation {
        invocation(kind, Path::new("agent.exe"), prompt, Path::new("C:/work"), Path::new("C:/scratch"))
            .unwrap()
    }

    #[test]
    fn every_invocation_asks_for_read_only_investigation() {
        let read_only = |i: &Invocation| {
            let a = i.args.join(" ");
            a.contains("--sandbox read-only")
                || a.contains("--permission-mode plan")
                || a.contains("--approval-mode plan")
                || a.contains("--mode plan")
        };
        for kind in [AgentKind::Codex, AgentKind::ClaudeCode, AgentKind::Gemini, AgentKind::GithubCopilot] {
            assert!(read_only(&build(kind, "p")), "{kind:?}");
        }
    }

    #[test]
    fn no_invocation_ever_bypasses_the_agents_own_safeguards() {
        for kind in AgentKind::ALL {
            let args = build(kind, "p").args.join(" ").to_ascii_lowercase();
            for forbidden in
                ["bypass", "dangerously", "yolo", "--allow-all", "full-access", "acceptedits", "autopilot"]
            {
                assert!(!args.contains(forbidden), "{kind:?} uses {forbidden}");
            }
        }
    }

    #[test]
    fn long_prompts_go_on_stdin_and_are_refused_where_they_cannot() {
        let long = "x".repeat(MAX_ARGUMENT_PROMPT + 1);
        assert_eq!(build(AgentKind::ClaudeCode, &long).stdin.len(), long.len());
        assert!(
            invocation(AgentKind::GithubCopilot, Path::new("c"), &long, Path::new("w"), Path::new("s"))
                .is_err()
        );
    }

    #[test]
    fn codex_writes_its_answer_to_a_file_in_the_scratch_directory() {
        let i = build(AgentKind::Codex, "p");
        assert_eq!(i.output_file, Some(PathBuf::from("C:/scratch").join("codex-last-message.txt")));
        assert_eq!(i.args.last().map(String::as_str), Some("-"), "prompt on stdin");
    }

    #[test]
    fn names_parse_leniently() {
        assert_eq!(AgentKind::from_name("Claude Code"), Some(AgentKind::ClaudeCode));
        assert_eq!(AgentKind::from_name("copilot"), Some(AgentKind::GithubCopilot));
        assert_eq!(AgentKind::from_name("chatgpt"), None);
    }
}
