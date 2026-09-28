//! Asking PowerShell about staged commands without running them, and the one
//! kind of dry run KeyJutsu trusts.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::process::{self, RunError, base64_decode, base64_encode};

const ANALYSE: &str = include_str!("analyse.ps1");
const ANALYSIS_LIMIT: Duration = Duration::from_secs(60);
const WHAT_IF_LIMIT: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize)]
struct Request<'a> {
    commands: Vec<CommandText<'a>>,
    tools: Vec<&'a str>,
    services: Vec<&'a str>,
}

#[derive(Debug, Serialize)]
struct CommandText<'a> {
    id: String,
    text: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SyntaxError {
    pub message: String,
    pub line: u32,
    pub column: u32,
}

/// One command name found in a command line.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CommandUse {
    pub name: Option<String>,
    /// `Cmdlet`, `Function`, `Alias`, `Application`, `ExternalScript`, or
    /// `None` when nothing by that name exists.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub module: Option<String>,
    pub path: Option<String>,
    pub file_version: Option<String>,
    #[serde(default)]
    pub arguments: Vec<String>,
    pub static_arguments: bool,
    #[serde(default)]
    pub parameters_used: Vec<String>,
    #[serde(default)]
    pub parameters_resolved: Vec<String>,
    #[serde(default)]
    pub unknown_parameters: Vec<String>,
    #[serde(default)]
    pub ambiguous_parameters: Vec<String>,
    pub supports_what_if: bool,
    /// Something earlier in the pipeline feeds it: it acts on whatever that
    /// yields, which the command line alone does not show.
    #[serde(default)]
    pub from_pipeline: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LineAnalysis {
    pub id: String,
    #[serde(default)]
    pub syntax_errors: Vec<SyntaxError>,
    pub single_command: bool,
    /// `exit` on the line itself, which ends the shell rather than the step.
    #[serde(default)]
    pub exits_shell: bool,
    #[serde(default)]
    pub commands: Vec<CommandUse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ToolInfo {
    pub path: String,
    pub file_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Analysis {
    pub edition: String,
    pub version: String,
    pub elevated: bool,
    #[serde(default)]
    pub results: Vec<LineAnalysis>,
    #[serde(default)]
    pub tools: BTreeMap<String, Option<ToolInfo>>,
    #[serde(default)]
    pub services: BTreeMap<String, String>,
}

impl Analysis {
    pub fn line(&self, id: &str) -> Option<&LineAnalysis> {
        self.results.iter().find(|r| r.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnalysisError {
    #[error(transparent)]
    Run(#[from] RunError),
    #[error("the analysis shell failed: {0}")]
    Failed(String),
    #[error("the analysis shell's answer could not be read: {0}")]
    Unreadable(String),
    #[error("not dry-run: {0}")]
    NotDryRunnable(&'static str),
}

fn powershell(program: &Path, script: &str) -> Command {
    let mut c = keyjutsu_terminal::shell::command(program);
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand"]);
    c.arg(keyjutsu_terminal::shell::encode_powershell_command(script));
    c
}

/// Analyse `lines` (id, text) in the PowerShell at `program`, and look up
/// `tools` and `services` while there. Nothing in `lines` is executed.
pub fn analyse(
    program: &Path,
    lines: &[(String, &str)],
    tools: &[&str],
    services: &[&str],
) -> Result<Analysis, AnalysisError> {
    let request = Request {
        commands: lines.iter().map(|(id, text)| CommandText { id: id.clone(), text }).collect(),
        tools: tools.to_vec(),
        services: services.to_vec(),
    };
    let json = serde_json::to_string(&request).map_err(|e| AnalysisError::Unreadable(e.to_string()))?;
    let done = process::run(powershell(program, ANALYSE), &base64_encode(json.as_bytes()), ANALYSIS_LIMIT)?;
    if !done.success {
        return Err(AnalysisError::Failed(done.stderr.trim().chars().take(500).collect()));
    }
    let bytes = base64_decode(done.stdout.trim())
        .ok_or_else(|| AnalysisError::Unreadable("the response was not base64".into()))?;
    serde_json::from_slice(&bytes).map_err(|e| AnalysisError::Unreadable(e.to_string()))
}

/// The one module whose cmdlets' `-WhatIf` KeyJutsu relies on. Its cmdlets
/// are compiled, ship with PowerShell, and route every change through
/// ShouldProcess; a function or third-party cmdlet may declare `-WhatIf` and
/// still act.
pub const TRUSTED_WHAT_IF_MODULE: &str = "Microsoft.PowerShell.Management";

/// Why a line cannot be dry-run, or `None` if it can.
pub fn what_if_blocker(line: &LineAnalysis) -> Option<&'static str> {
    if !line.syntax_errors.is_empty() {
        return Some("it does not parse");
    }
    if !line.single_command {
        return Some("it is more than one command, and -WhatIf covers only one");
    }
    let c = line.commands.first()?;
    if c.kind.as_deref() != Some("Cmdlet") || c.module.as_deref() != Some(TRUSTED_WHAT_IF_MODULE) {
        return Some("only built-in management cmdlets are trusted to honour -WhatIf");
    }
    if !c.supports_what_if {
        return Some("the cmdlet has no -WhatIf");
    }
    if !c.static_arguments {
        return Some("its arguments contain expressions, which a dry run would evaluate");
    }
    if c.parameters_resolved.iter().any(|p| p == "WhatIf" || p == "Confirm") {
        return Some("it already sets -WhatIf or -Confirm");
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhatIf {
    pub ran: bool,
    /// The "What if:" lines, which name the exact targets, with wildcards and
    /// relative paths expanded.
    pub operations: Vec<String>,
    pub errors: Vec<String>,
}

/// Run `text` in a throwaway shell with `$WhatIfPreference` set for the whole
/// session. Only call this for a line that [`what_if_blocker`] clears.
///
/// The preference is set, rather than ` -WhatIf` appended to the text,
/// because appending does nothing when the line ends in a comment:
/// `Remove-Item x # note -WhatIf` would really delete `x`. The preference
/// covers the command however it is written, and the blocker has already
/// refused any line that sets `-WhatIf` itself.
pub fn what_if(program: &Path, line: &LineAnalysis, text: &str) -> Result<WhatIf, AnalysisError> {
    // Checked here as well as by callers: this is the only function in
    // validation that runs anything a plan contains.
    if let Some(reason) = what_if_blocker(line) {
        return Err(AnalysisError::NotDryRunnable(reason));
    }
    if text.chars().any(char::is_control) {
        return Err(AnalysisError::NotDryRunnable("it contains a control character"));
    }
    let script = format!(
        "$WhatIfPreference = $true\n$ErrorActionPreference = 'Continue'\n$WarningPreference = 'SilentlyContinue'\n$ProgressPreference = 'SilentlyContinue'\n{text}"
    );
    let done = process::run(powershell(program, &script), "", WHAT_IF_LIMIT)?;
    let mut operations = Vec::new();
    let mut errors = Vec::new();
    for line in done.stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with("What if:") {
            operations.push(line.to_owned());
        } else {
            errors.push(line.to_owned());
        }
    }
    errors.extend(
        done.stderr
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !is_progress_only(l))
            .map(readable_error)
            .filter(|l| !l.is_empty()),
    );
    Ok(WhatIf { ran: true, operations, errors })
}

/// PowerShell with redirected output writes progress (such as "preparing
/// modules for first use") to standard error as CLIXML. That is not an
/// error: a clean dry run was being reported as failed whenever it happened.
/// A CLIXML document is ignored only if every record in it is progress.
pub fn is_progress_only(line: &str) -> bool {
    if line == "#< CLIXML" {
        return true;
    }
    if !line.starts_with("<Objs") {
        return false;
    }
    let kinds: Vec<&str> = line.split("<Obj S=\"").skip(1).filter_map(|r| r.split('"').next()).collect();
    !kinds.is_empty() && kinds.iter().all(|k| *k == "progress")
}

/// An error as PowerShell would have shown it. With its output redirected,
/// PowerShell writes errors to standard error as CLIXML: the rendered error,
/// colour codes included, split into `<S S="Error">` strings with control
/// characters escaped as `_xHHHH_`. What people need is its first line (the
/// command) and its last (the message); the lines between point at the
/// column. Anything that isn't CLIXML is returned as it came.
pub fn readable_error(line: &str) -> String {
    if !line.starts_with("<Objs") {
        return line.to_owned();
    }
    let text: String = line.split("<S S=\"Error\">").skip(1).filter_map(|s| s.split("</S>").next()).collect();
    let text = unescape_clixml(&text);
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim().trim_start_matches('|').trim())
        .filter(|l| !l.is_empty() && !l.chars().all(|c| c == '~'))
        .collect();
    match (lines.first(), lines.last()) {
        (Some(first), Some(last)) if lines.len() > 1 && first.ends_with(':') => format!("{first} {last}"),
        (Some(_), Some(last)) => (*last).to_owned(),
        _ => String::new(),
    }
}

/// `_xHHHH_` escapes decoded, colour codes (ESC [ ... letter) dropped, and
/// the XML entities put back.
fn unescape_clixml(text: &str) -> String {
    let mut decoded = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("_x") {
        decoded.push_str(&rest[..i]);
        let tail = &rest[i..];
        let code = tail
            .get(2..6)
            .filter(|_| tail.get(6..7) == Some("_"))
            .and_then(|h| u32::from_str_radix(h, 16).ok());
        match code.and_then(char::from_u32) {
            Some(c) => {
                decoded.push(c);
                rest = &tail[7..];
            }
            None => {
                decoded.push_str("_x");
                rest = &tail[2..];
            }
        }
    }
    decoded.push_str(rest);
    let mut plain = String::new();
    let mut chars = decoded.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if c != '\r' {
            plain.push(c);
        }
    }
    plain
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_on_standard_error_is_not_an_error_but_an_error_record_is() {
        // Seen for real: a clean dry run failed on this.
        let progress = r#"<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04"><Obj S="progress" RefId="0"><TN RefId="0"><T>System.Management.Automation.PSCustomObject</T><T>System.Object</T></TN><MS><I64 N="SourceId">1</I64><PR N="Record"><AV> </AV><AI>0</AI><Nil /><PI>-1</PI><PC>-1</PC><T>Completed</T><SR>-1</SR><SD> </SD></PR></MS></Obj></Objs>"#;
        assert!(is_progress_only("#< CLIXML"));
        assert!(is_progress_only(progress));
        let error = r#"<Objs Version="1.1.0.1"><Obj S="progress" RefId="0"></Obj><S S="Error">Cannot find path</S><Obj S="Error" RefId="1"></Obj></Objs>"#;
        assert!(!is_progress_only(error));
        assert!(!is_progress_only("Remove-Item: Cannot find path"));
    }

    /// A failed dry run's reason is PowerShell's message, not the raw CLIXML
    /// with its escaped colour codes.
    #[test]
    fn a_dry_run_error_reads_as_powershell_would_show_it() {
        let raw = r#"<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04"><S S="Error">_x001B_[31;1mRemove-Item: _x001B_[0m_x000D__x000A_</S><S S="Error">_x001B_[31;1m_x001B_[36;1mLine |_x001B_[0m_x000D__x000A_</S><S S="Error">_x001B_[31;1m_x001B_[36;1m_x001B_[36;1m   5 | _x001B_[0m _x001B_[36;1mRemove-Item -Recurse -Force -LiteralPath C:/Users/Public/BuildCache_x001B_[0m_x000D__x000A_</S><S S="Error">_x001B_[31;1m_x001B_[36;1m_x001B_[36;1m_x001B_[0m_x001B_[36;1m_x001B_[0m_x001B_[36;1m     | _x001B_[31;1m ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~_x001B_[0m_x000D__x000A_</S><S S="Error">_x001B_[31;1m_x001B_[36;1m_x001B_[36;1m_x001B_[0m_x001B_[36;1m_x001B_[0m_x001B_[36;1m_x001B_[31;1m_x001B_[31;1m_x001B_[36;1m     | _x001B_[31;1mCannot find path 'C:/Users/Public/BuildCache' because it does not exist._x001B_[0m_x000D__x000A_</S></Objs>"#;
        assert_eq!(
            readable_error(raw),
            "Remove-Item: Cannot find path 'C:/Users/Public/BuildCache' because it does not exist."
        );
        let entities = r#"<Objs Version="1.1.0.1"><S S="Error">Get-Item: a &lt;b&gt; &amp; &quot;c&quot;_x000D__x000A_</S></Objs>"#;
        assert_eq!(readable_error(entities), r#"Get-Item: a <b> & "c""#);
        assert_eq!(readable_error("plain text stays"), "plain text stays");
    }

    fn line(json: serde_json::Value) -> LineAnalysis {
        serde_json::from_value(json).unwrap()
    }

    fn cmdlet(module: &str, what_if: bool, static_args: bool) -> serde_json::Value {
        serde_json::json!({
            "id": "x", "single_command": true, "syntax_errors": [],
            "commands": [{
                "name": "Remove-Item", "type": "Cmdlet", "module": module, "static_arguments": static_args,
                "supports_what_if": what_if, "parameters_resolved": ["Recurse"]
            }]
        })
    }

    #[test]
    fn only_static_built_in_management_cmdlets_are_dry_run() {
        assert_eq!(what_if_blocker(&line(cmdlet(TRUSTED_WHAT_IF_MODULE, true, true))), None);
        assert!(what_if_blocker(&line(cmdlet("SomeVendor.Module", true, true))).is_some());
        assert!(what_if_blocker(&line(cmdlet(TRUSTED_WHAT_IF_MODULE, false, true))).is_some());
        assert!(what_if_blocker(&line(cmdlet(TRUSTED_WHAT_IF_MODULE, true, false))).is_some(), "expressions");
        let mut multi = cmdlet(TRUSTED_WHAT_IF_MODULE, true, true);
        multi["single_command"] = false.into();
        assert!(what_if_blocker(&line(multi)).is_some());
        let mut confirm = cmdlet(TRUSTED_WHAT_IF_MODULE, true, true);
        confirm["commands"][0]["parameters_resolved"] = serde_json::json!(["WhatIf"]);
        assert!(what_if_blocker(&line(confirm)).is_some(), "-WhatIf:$false must not be overridden");
    }
}
