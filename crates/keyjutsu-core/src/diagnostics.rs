//! A diagnostic bundle: what someone helping with a problem needs to know
//! about this machine and this copy of KeyJutsu, as plain text the operator
//! reads in full before deciding to send it anywhere.
//!
//! KeyJutsu sends nothing itself. The bundle is built from the readiness
//! scan, the installed agents and counts from the history, and nothing else:
//! no task, plan, command, terminal contents, run output or environment
//! variable is read into it. What it does hold is scrubbed on the way out.
//! The profile folder, the Windows user name and the computer name are
//! replaced with placeholders, and the whole text is put through the same
//! redaction as agent context, so a token in, say, a terminal profile's
//! command line is removed rather than shared.

use std::fmt::Write as _;

use keyjutsu_agent::AgentInfo;
use keyjutsu_agent::detect::SignIn;
use regex::Regex;

use crate::history::SessionSummary;
use crate::readiness::{CheckStatus, ReadinessReport};

/// What the history holds, as counts. Either may be unreadable, for example
/// when the store's key cannot be unwrapped; the bundle says so.
#[derive(Debug)]
pub struct Kept {
    pub sessions: Result<Vec<SessionSummary>, String>,
    pub techniques: Result<usize, String>,
}

/// The identifying values replaced in the bundle, and what replaces them.
#[derive(Debug, Clone, Default)]
pub struct Scrub {
    pub profile_dir: Option<String>,
    pub user: Option<String>,
    pub computer: Option<String>,
}

impl Scrub {
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self { profile_dir: var("USERPROFILE"), user: var("USERNAME"), computer: var("COMPUTERNAME") }
    }

    /// The text with every identifying value replaced, then redacted.
    pub fn apply(&self, text: &str) -> String {
        let mut out = text.to_owned();
        // The profile folder first: it usually contains the user name, and
        // `%USERPROFILE%` says more than `C:\Users\<user>` would.
        if let Some(dir) = &self.profile_dir {
            let dir = dir.trim_end_matches(['\\', '/']);
            if let Ok(re) = Regex::new(&format!("(?i){}", regex::escape(dir))) {
                out = re.replace_all(&out, "%USERPROFILE%").into_owned();
            }
        }
        for (value, placeholder) in [(&self.user, "<user>"), (&self.computer, "<computer>")] {
            // Whole words only, so a short name does not eat into others.
            if let Some(v) = value.as_deref().filter(|v| v.chars().count() >= 2)
                && let Ok(re) = Regex::new(&format!(r"(?i)\b{}\b", regex::escape(v)))
            {
                out = re.replace_all(&out, placeholder).into_owned();
            }
        }
        keyjutsu_agent::context::redact(&out).0
    }
}

/// The bundle's full text: exactly what a preview shows and a save writes.
pub fn bundle(report: &ReadinessReport, agents: &[AgentInfo], kept: &Kept, scrub: &Scrub) -> String {
    let mut t = String::new();
    let _ = writeln!(t, "KeyJutsu diagnostic bundle");
    let _ = writeln!(t);
    let _ =
        writeln!(t, "This is all of it. KeyJutsu has not sent it anywhere; it goes only where you send it.");
    let _ = writeln!(
        t,
        "Left out: tasks, plans, commands, terminal contents, what runs printed, environment variables and credentials."
    );
    let _ = writeln!(
        t,
        "Replaced: your profile folder with %USERPROFILE%, your Windows user name with <user>, this computer's name with <computer>, and anything shaped like a secret with [REDACTED]."
    );
    let _ = writeln!(t);

    let w = &report.windows;
    let _ = writeln!(t, "KeyJutsu       {}", report.keyjutsu_version);
    let _ = writeln!(
        t,
        "Windows        {}{}{}",
        w.product,
        w.display_version.as_deref().map(|v| format!(" {v}")).unwrap_or_default(),
        w.build.as_deref().map(|b| format!(" (build {b})")).unwrap_or_default()
    );
    let _ = writeln!(t, "Architecture   {}", report.architecture);
    let _ = writeln!(t);

    let _ = writeln!(t, "Checks");
    for c in &report.checks {
        let mark = match c.status {
            CheckStatus::Ok => "ok  ",
            CheckStatus::Warning => "warn",
            CheckStatus::Unavailable => "fail",
        };
        let _ = writeln!(t, "  [{mark}] {:<36} {}", c.name, c.detail);
    }
    let _ = writeln!(t);

    let _ = writeln!(t, "Shells");
    for s in &report.shells {
        let _ = writeln!(
            t,
            "  {:<24} {:<18} {}",
            s.kind.display_name(),
            s.version.as_deref().unwrap_or("version unknown"),
            s.path
        );
    }
    let _ = writeln!(t);

    // The pseudo-console probes: how each shell behaved when KeyJutsu
    // started it and typed into it.
    let _ = writeln!(t, "Terminal probes");
    for p in &report.probes {
        let ready = match (p.ready, p.ready_ms) {
            (true, Some(ms)) => format!("prompt in {ms} ms"),
            (true, None) => "prompt drawn".to_owned(),
            (false, _) => "no prompt".to_owned(),
        };
        let exit = p.exit_code.map(|c| format!(", exit code {c}")).unwrap_or_default();
        let _ = writeln!(
            t,
            "  {:<24} {ready}, staged typing {}{exit}: {}",
            p.kind.display_name(),
            if p.staged_typing { "works" } else { "failed" },
            p.detail
        );
    }
    let _ = writeln!(t);

    let tp = &report.terminal_profile;
    let _ = writeln!(t, "Terminal profile");
    let _ = writeln!(t, "  from           {}", tp.source);
    let _ = writeln!(t, "  profile        {}", tp.name);
    if let Some(cmd) = &tp.commandline {
        let _ = writeln!(t, "  command line   {cmd}");
    }
    let _ = writeln!(t, "  font           {} {}", tp.font_face, tp.font_size);
    if let Some(ps) = &report.powershell_profile {
        let _ = writeln!(t);
        let _ = writeln!(t, "PowerShell profile");
        if ps.scripts.is_empty() {
            let _ = writeln!(t, "  no profile scripts");
        }
        for s in &ps.scripts {
            let _ = writeln!(t, "  script         {s}");
        }
        let _ = writeln!(
            t,
            "  oh-my-posh {}, Starship {}, PSReadLine customised {}",
            yes_no(ps.uses_oh_my_posh),
            yes_no(ps.uses_starship),
            yes_no(ps.configures_psreadline)
        );
    }
    let _ = writeln!(t);

    let _ = writeln!(t, "Agents");
    for a in agents {
        if !a.installed() {
            let _ = writeln!(t, "  {:<19} not installed", a.name);
            continue;
        }
        let sign_in = match a.sign_in {
            SignIn::CredentialsFound => "credentials found",
            SignIn::NoCredentialsFound => "no credentials found",
            SignIn::Unknown => "sign-in not checked",
        };
        let checked = if a.needs_compatibility_check {
            "not the version the adapter was checked against"
        } else {
            "the version the adapter was checked against"
        };
        let _ = writeln!(
            t,
            "  {:<19} {:<16} {sign_in}; {checked}",
            a.name,
            a.version.as_deref().unwrap_or("version unknown")
        );
    }
    let _ = writeln!(t);

    // Counts only: a session's task and steps say what the operator was
    // doing, which is theirs to share or not.
    let _ = writeln!(t, "History");
    match &kept.sessions {
        Ok(sessions) => {
            let complete = sessions.iter().filter(|s| s.outcome == "complete").count();
            let failed = sessions.iter().filter(|s| s.outcome.starts_with("failed")).count();
            let disarmed = sessions.iter().filter(|s| s.outcome == "disarmed").count();
            let other = sessions.len() - complete - failed - disarmed;
            let _ = writeln!(
                t,
                "  {} recorded: {complete} complete, {failed} failed, {disarmed} disarmed, {other} stopped otherwise",
                crate::plan::count(sessions.len(), "run", "runs")
            );
        }
        Err(e) => {
            let _ = writeln!(t, "  could not be read: {e}");
        }
    }
    match &kept.techniques {
        Ok(n) => {
            let _ = writeln!(t, "  {} saved", crate::plan::count(*n, "Technique", "Techniques"));
        }
        Err(e) => {
            let _ = writeln!(t, "  Techniques could not be read: {e}");
        }
    }
    scrub.apply(&t)
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// Everything the bundle is built from, gathered from this machine.
pub fn collect() -> String {
    let report = crate::readiness::scan();
    let agents = keyjutsu_agent::detect_all();
    let kept = match crate::store::Store::open(&crate::store::default_root()) {
        Ok(store) => Kept {
            sessions: crate::history::list(&store),
            techniques: crate::technique::list(&store).map(|t| t.len()),
        },
        Err(e) => Kept { sessions: Err(e.clone()), techniques: Err(e) },
    };
    bundle(&report, &agents, &kept, &Scrub::from_env())
}

/// Where the desktop app saves bundles: beside KeyJutsu's other data, so
/// clearing KeyJutsu's folder clears them too.
pub fn default_dir() -> std::path::PathBuf {
    crate::store::default_root().with_file_name("diagnostics")
}

/// Write a bundle the operator has already seen to a new, dated file in
/// `dir`, and say where.
pub fn save(text: &str, dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let stamp = crate::fingerprint::now_rfc3339().replace(':', "");
    let file = dir.join(format!("keyjutsu-diagnostics-{stamp}.txt"));
    std::fs::write(&file, text).map_err(|e| format!("could not write {}: {e}", file.display()))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readiness::{Check, ShellProbe, WindowsInfo};
    use keyjutsu_terminal::profile::{PowerShellProfileReport, TerminalProfile};
    use keyjutsu_terminal::{ProfileMode, ShellInfo, ShellKind};

    const PROFILE: &str = r"C:\Users\Robin.Hale";

    fn report() -> ReadinessReport {
        let mut terminal_profile = TerminalProfile::built_in();
        terminal_profile.source = format!(
            r"{PROFILE}\AppData\Local\Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json"
        );
        terminal_profile.commandline = Some(
            "pwsh.exe -NoExit -Command $env:GITHUB_TOKEN='ghp_0123456789abcdefghijklmnopqrstuvwxyzAB'".into(),
        );
        ReadinessReport {
            keyjutsu_version: "0.1.0".into(),
            windows: WindowsInfo {
                product: "Windows 11 Pro".into(),
                display_version: Some("24H2".into()),
                build: Some("26100.1".into()),
            },
            architecture: "x64".into(),
            shells: vec![ShellInfo {
                kind: ShellKind::Pwsh,
                path: format!(r"{PROFILE}\AppData\Local\Microsoft\WindowsApps\pwsh.exe"),
                version: Some("7.5.0".into()),
            }],
            probes: vec![ShellProbe {
                kind: ShellKind::Pwsh,
                profile: ProfileMode::Detected,
                ready: true,
                ready_ms: Some(300),
                staged_typing: true,
                exit_code: Some(0),
                detail: "staged typing and exit codes both work".into(),
            }],
            terminal_profile,
            powershell_profile: Some(PowerShellProfileReport {
                scripts: vec![format!(r"{PROFILE}\Documents\PowerShell\Microsoft.PowerShell_profile.ps1")],
                uses_oh_my_posh: true,
                uses_starship: false,
                configures_psreadline: false,
            }),
            checks: vec![Check {
                name: "Windows".into(),
                status: CheckStatus::Ok,
                detail: "running on ROBINS-DESK as robin.hale".into(),
                get: None,
            }],
        }
    }

    fn scrub() -> Scrub {
        Scrub {
            profile_dir: Some(PROFILE.into()),
            user: Some("Robin.Hale".into()),
            computer: Some("ROBINS-DESK".into()),
        }
    }

    fn kept(outcomes: &[&str]) -> Kept {
        Kept {
            sessions: Ok(outcomes
                .iter()
                .enumerate()
                .map(|(i, o)| SessionSummary {
                    id: format!("s{i}"),
                    finished_at: "2026-01-01T00:00:00Z".into(),
                    task: "Move the client's private files".into(),
                    outcome: (*o).into(),
                })
                .collect()),
            techniques: Ok(1),
        }
    }

    #[test]
    fn the_bundle_names_no_one_and_holds_no_secret() {
        let text = bundle(&report(), &[], &kept(&["complete"]), &scrub());
        for leak in ["Robin", "robin", "ROBINS-DESK", "ghp_0123456789", "client's private files"] {
            assert!(!text.contains(leak), "{leak} is in the bundle:\n{text}");
        }
        assert!(text.contains(r"%USERPROFILE%\AppData\Local\Microsoft\WindowsApps\pwsh.exe"), "{text}");
        assert!(text.contains("running on <computer> as <user>"), "{text}");
        assert!(text.contains("[REDACTED"), "{text}");
    }

    #[test]
    fn the_bundle_carries_what_helps_diagnose() {
        let text = bundle(&report(), &[], &kept(&["complete", "failed at svc", "disarmed"]), &scrub());
        for want in [
            "KeyJutsu       0.1.0",
            "Windows 11 Pro 24H2 (build 26100.1)",
            "PowerShell 7",
            "prompt in 300 ms, staged typing works, exit code 0",
            "oh-my-posh yes",
            "3 runs recorded: 1 complete, 1 failed, 1 disarmed, 0 stopped otherwise",
            "1 Technique saved",
        ] {
            assert!(text.contains(want), "missing {want}:\n{text}");
        }
    }

    #[test]
    fn a_short_user_name_is_replaced_only_as_a_word() {
        let s = Scrub { profile_dir: None, user: Some("al".into()), computer: None };
        assert_eq!(s.apply("al ran it in the terminal"), "<user> ran it in the terminal");
    }

    #[test]
    fn a_saved_bundle_is_exactly_the_text_that_was_shown() {
        let dir = std::env::temp_dir().join(format!("kj-diagnostics-{}", std::process::id()));
        let shown = bundle(&report(), &[], &kept(&[]), &scrub());
        let file = save(&shown, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), shown);
        assert!(file.file_name().unwrap().to_string_lossy().starts_with("keyjutsu-diagnostics-"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_history_is_said_rather_than_hidden() {
        let k =
            Kept { sessions: Err("the key could not be unwrapped".into()), techniques: Err("same".into()) };
        let text = bundle(&report(), &[], &k, &scrub());
        assert!(text.contains("could not be read: the key could not be unwrapped"), "{text}");
    }
}
