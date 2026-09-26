//! Locating shells and launching them with KeyJutsu's shell integration.
//!
//! A KeyJutsu command is never shell-agnostic, so the shell is always a
//! concrete executable at a concrete path with a detected version.

use std::path::{Path, PathBuf};
use std::process::Command;

use portable_pty::CommandBuilder;
use serde::{Deserialize, Serialize};

use crate::marks::Nonce;

/// The shells V1 executes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ShellKind {
    /// PowerShell 7 (`pwsh.exe`). Preferred where available.
    Pwsh,
    /// Windows PowerShell 5.1 (`powershell.exe`).
    WindowsPowershell,
    /// `cmd.exe`.
    Cmd,
}

impl ShellKind {
    pub const ALL: [ShellKind; 3] = [ShellKind::Pwsh, ShellKind::WindowsPowershell, ShellKind::Cmd];

    pub fn display_name(self) -> &'static str {
        match self {
            ShellKind::Pwsh => "PowerShell 7",
            ShellKind::WindowsPowershell => "Windows PowerShell 5.1",
            ShellKind::Cmd => "Command Prompt",
        }
    }

    /// Whether the shell integration can report a command's exit code.
    /// `cmd.exe` cannot: its `PROMPT` is not re-evaluated for variables, so
    /// `%ERRORLEVEL%` is unavailable at the point the mark is drawn.
    pub fn reports_exit_codes(self) -> bool {
        !matches!(self, ShellKind::Cmd)
    }
}

/// Whether the user's own shell profile is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProfileMode {
    /// Load the user's profile (`$PROFILE`, or cmd's AutoRun), as their own
    /// terminal would.
    #[default]
    Detected,
    /// Skip it: `-NoProfile` for PowerShell, `/D` for cmd. The fallback when
    /// a profile interferes with staged input.
    Clean,
}

/// A shell found on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct ShellInfo {
    pub kind: ShellKind,
    pub path: String,
    pub version: Option<String>,
}

/// Find a shell's executable. Returns `None` when it is not installed.
pub fn locate(kind: ShellKind) -> Option<PathBuf> {
    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from);
    match kind {
        ShellKind::Pwsh => {
            // A Store PowerShell puts its own package folder first on PATH,
            // but only for programs started from it; every other program
            // finds the alias in WindowsApps. Both are the same PowerShell, so
            // the package folder is passed over: otherwise this machine would
            // look different depending on what started KeyJutsu.
            let pf = std::env::var_os("ProgramFiles").map(PathBuf::from);
            let packages = pf.as_ref().map(|pf| pf.join("WindowsApps"));
            let path = std::env::var_os("PATH").unwrap_or_default();
            first_on_path(std::env::split_paths(&path), "pwsh.exe", packages.as_deref()).or_else(|| {
                let p = pf?.join("PowerShell").join("7").join("pwsh.exe");
                p.is_file().then_some(p)
            })
        }
        ShellKind::WindowsPowershell => {
            let p = system_root?.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
            p.is_file().then_some(p)
        }
        ShellKind::Cmd => {
            std::env::var_os("ComSpec").map(PathBuf::from).filter(|p| p.is_file()).or_else(|| {
                let p = system_root?.join(r"System32\cmd.exe");
                p.is_file().then_some(p)
            })
        }
    }
}

/// The first `exe` in `dirs`, skipping any folder inside `except`.
fn first_on_path(dirs: impl Iterator<Item = PathBuf>, exe: &str, except: Option<&Path>) -> Option<PathBuf> {
    let inside = |dir: &Path| {
        except.is_some_and(|e| {
            let (dir, e) = (dir.to_string_lossy().to_lowercase(), e.to_string_lossy().to_lowercase());
            dir.strip_prefix(e.trim_end_matches('\\'))
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('\\'))
        })
    };
    dirs.filter(|dir| !inside(dir)).map(|dir| dir.join(exe)).find(|p| p.is_file())
}

/// Ask the shell for its version. This starts the shell once, without a
/// profile, so it costs a few hundred milliseconds per PowerShell.
pub fn detect_version(kind: ShellKind, program: &Path) -> Option<String> {
    let output = match kind {
        ShellKind::Pwsh | ShellKind::WindowsPowershell => Command::new(program)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$PSVersionTable.PSVersion.ToString()",
            ])
            .output()
            .ok()?,
        ShellKind::Cmd => Command::new(program).args(["/D", "/C", "ver"]).output().ok()?,
    };
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    match kind {
        // "Microsoft Windows [Version 10.0.26200.9457]"
        ShellKind::Cmd => text.split("[Version ").nth(1).and_then(|s| s.split(']').next()).map(str::to_owned),
        _ => text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_owned),
    }
}

/// Every supported shell present on this machine, with its version.
pub fn detect_all() -> Vec<ShellInfo> {
    ShellKind::ALL
        .into_iter()
        .filter_map(|kind| {
            let path = locate(kind)?;
            Some(ShellInfo { kind, version: detect_version(kind, &path), path: path.display().to_string() })
        })
        .collect()
}

/// Everything needed to start one shell inside a pseudo-console.
#[derive(Debug, Clone)]
pub struct ShellLaunch {
    pub kind: ShellKind,
    pub program: PathBuf,
    pub profile: ProfileMode,
    pub cwd: Option<PathBuf>,
    pub nonce: Nonce,
}

impl ShellLaunch {
    /// Locate `kind` and prepare to launch it with a fresh nonce.
    pub fn for_kind(kind: ShellKind) -> crate::Result<Self> {
        let program = locate(kind)
            .ok_or_else(|| crate::TerminalError::ShellNotFound(kind.display_name().to_owned()))?;
        Ok(Self { kind, program, profile: ProfileMode::Detected, cwd: None, nonce: Nonce::generate()? })
    }

    pub fn with_profile(mut self, profile: ProfileMode) -> Self {
        self.profile = profile;
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn command_builder(&self) -> CommandBuilder {
        let mut cmd = CommandBuilder::new(&self.program);
        match self.kind {
            ShellKind::Pwsh | ShellKind::WindowsPowershell => {
                cmd.arg("-NoLogo");
                if self.profile == ProfileMode::Clean {
                    cmd.arg("-NoProfile");
                }
                cmd.arg("-NoExit");
                // Encoded so no layer of Windows command-line quoting can
                // alter the script on its way in.
                cmd.arg("-EncodedCommand");
                cmd.arg(encode_powershell_command(&powershell_integration(&self.nonce, self.profile)));
            }
            ShellKind::Cmd => {
                if self.profile == ProfileMode::Clean {
                    cmd.arg("/D");
                }
                let original = std::env::var("PROMPT").unwrap_or_else(|_| "$P$G".to_owned());
                cmd.env("PROMPT", cmd_prompt(&self.nonce, &original));
            }
        }
        let cwd = self.cwd.clone().or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from));
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }
        cmd
    }
}

/// The prompt wrapper for both PowerShell editions.
///
/// It runs after the user's profile, so it wraps whatever prompt the profile
/// installed (oh-my-posh, Starship or a hand-written one). `$?` is read first,
/// before anything else can reset it, and then put back with a suppressed
/// `Write-Error` so the wrapped prompt still sees the real success state. The
/// exit code is the native one where a native command failed, otherwise 1;
/// after a cmdlet failure that follows an earlier native failure the code can
/// be stale, which is why the engine treats success versus failure, not the
/// number, as the signal.
///
/// With the clean profile it also turns off PSReadLine's history predictions
/// and stops history being saved. Predictions draw earlier commands from the
/// user's history file as grey text while a line is typed, which during a
/// performance can put anything the user once ran on screen; and commands run
/// in a clean session do not belong in their history. PSReadLine 2.0, which
/// ships with Windows PowerShell 5.1, has no predictions, hence the `try`.
pub fn powershell_integration(nonce: &Nonce, profile: ProfileMode) -> String {
    let n = nonce.as_str();
    let clean = if profile == ProfileMode::Clean {
        r#"if (Get-Module PSReadLine) {
    Set-PSReadLineOption -HistorySaveStyle SaveNothing
    try { Set-PSReadLineOption -PredictionSource None -ErrorAction Stop } catch {}
}
"#
    } else {
        ""
    };
    format!(
        r#"{clean}$global:__KeyJutsuOriginalPrompt = $function:prompt
function global:prompt {{
    $kjOk = $global:?
    $kjCode = if ($kjOk) {{ 0 }} elseif ($global:LASTEXITCODE -is [int] -and $global:LASTEXITCODE -ne 0) {{ $global:LASTEXITCODE }} else {{ 1 }}
    $kjE = [char]27; $kjB = [char]7
    if (-not $kjOk) {{ Write-Error 'keyjutsu' -ErrorAction Ignore }}
    $kjPrompt = (& $global:__KeyJutsuOriginalPrompt) -join ''
    $kjCwd = $executionContext.SessionState.Path.CurrentFileSystemLocation.ProviderPath
    "$kjE]133;D;$kjCode;kj={n}$kjB$kjE]133;P;kj={n};cwd=$kjCwd$kjB$kjE]133;A;kj={n}$kjB$kjPrompt$kjE]133;B;kj={n}$kjB"
}}"#
    )
}

/// cmd's `PROMPT` with marks around the user's original prompt. There is no
/// exit code in the `D` mark: see [`ShellKind::reports_exit_codes`].
pub fn cmd_prompt(nonce: &Nonce, original: &str) -> String {
    let n = nonce.as_str();
    format!(r"$e]133;D;kj={n}$e\$e]133;P;kj={n};cwd=$P$e\$e]133;A;kj={n}$e\{original}$e]133;B;kj={n}$e\")
}

/// `-EncodedCommand` takes base64 of the UTF-16LE script.
pub fn encode_powershell_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(TABLE[((n >> shift) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }

    #[test]
    fn a_store_powershells_own_folder_is_passed_over_for_the_alias_everyone_sees() {
        let root = std::env::temp_dir().join(format!("kj-path-{}", std::process::id()));
        let package = root.join("Program Files").join("WindowsApps").join("Microsoft.PowerShell_7.6.6.0_x64");
        let alias = root.join("WindowsApps");
        for dir in [&package, &alias] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("pwsh.exe"), b"").unwrap();
        }
        let packages = root.join("Program Files").join("WindowsApps");
        let dirs = || [package.clone(), alias.clone()].into_iter();
        assert_eq!(first_on_path(dirs(), "pwsh.exe", Some(&packages)), Some(alias.join("pwsh.exe")));
        // Windows paths ignore case, and so does the comparison.
        let shouted = PathBuf::from(packages.to_string_lossy().to_uppercase());
        assert_eq!(first_on_path(dirs(), "pwsh.exe", Some(&shouted)), Some(alias.join("pwsh.exe")));
        // Told nothing to skip, PATH order stands.
        assert_eq!(first_on_path(dirs(), "pwsh.exe", None), Some(package.join("pwsh.exe")));
        // A folder that only starts with the same letters is not inside it.
        let lookalike = root.join("Program Files").join("WindowsAppsX");
        std::fs::create_dir_all(&lookalike).unwrap();
        std::fs::write(lookalike.join("pwsh.exe"), b"").unwrap();
        let found =
            first_on_path([lookalike.clone(), alias.clone()].into_iter(), "pwsh.exe", Some(&packages));
        assert_eq!(found, Some(lookalike.join("pwsh.exe")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn encoded_command_is_utf16le_base64() {
        // "a" in UTF-16LE is 61 00.
        assert_eq!(encode_powershell_command("a"), "YQA=");
    }

    #[test]
    fn integration_scripts_carry_the_nonce_on_every_mark() {
        let nonce = Nonce::from_fixed("abc123");
        let ps = powershell_integration(&nonce, ProfileMode::Detected);
        // D, P (location), A, B: every mark carries the nonce.
        assert_eq!(ps.matches("kj=abc123").count(), 4);
        assert_eq!(ps.matches("]133;").count(), 4);
        assert!(!ps.contains("PredictionSource"), "a detected profile is left as the user set it");
        let clean = powershell_integration(&nonce, ProfileMode::Clean);
        assert!(clean.contains("-PredictionSource None") && clean.contains("SaveNothing"));
        let cmd = cmd_prompt(&nonce, "$P$G");
        assert_eq!(cmd.matches("kj=abc123").count(), 4);
        assert_eq!(cmd.matches("]133;").count(), 4);
        assert!(cmd.contains("$P$G"));
    }

    #[test]
    fn cmd_is_the_only_shell_without_exit_codes() {
        assert!(ShellKind::Pwsh.reports_exit_codes());
        assert!(ShellKind::WindowsPowershell.reports_exit_codes());
        assert!(!ShellKind::Cmd.reports_exit_codes());
    }
}
