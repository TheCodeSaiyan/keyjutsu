//! What an agent is told about this machine before it proposes a plan, so
//! the commands it writes target the versions and folders that are really
//! here instead of guessing (a first plan put a PDF on a Desktop that lives in
//! OneDrive only after the agent went looking).
//!
//! It is sent as one more context item: listed in the manifest the operator
//! sees before anything goes, and redacted like the rest. It names no user:
//! folders are given as `%USERPROFILE%\…`, as Windows stores them, and a
//! folder stored in full has the profile folder replaced.

use keyjutsu_plan::hash::FingerprintEntry;

use crate::fingerprint;

/// Programs worth telling an agent about when they are on PATH: what a plan
/// is likely to reach for.
pub const TOOLS: &[&str] = &[
    "git", "python", "py", "node", "npm", "winget", "choco", "scoop", "docker", "wsl", "dotnet", "java",
    "gh", "az", "code", "magick", "ffmpeg", "7z", "curl",
];

/// What KeyJutsu found, before it is put into words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub os: String,
    pub build: String,
    pub architecture: String,
    pub shells: Vec<FingerprintEntry>,
    pub elevated: bool,
    /// Desktop, Documents and Downloads, as `(name, where)`.
    pub folders: Vec<(String, String)>,
    pub tools: Vec<FingerprintEntry>,
}

/// This machine, found now. Starts each shell once for its version and asks
/// PowerShell for the tools' file versions; runs nothing else.
pub fn facts() -> Facts {
    let fp = fingerprint::collect(None);
    Facts {
        os: fp.os,
        build: fp.build,
        architecture: fp.architecture,
        shells: fp.shells,
        elevated: crate::elevation::is_elevated(),
        folders: known_folders(),
        tools: fingerprint::tools(TOOLS.iter().map(|t| (*t).to_owned()).collect()),
    }
}

/// A version as an agent can use it: the number a file's version resource
/// starts with (`10.0.12 @Commit: …` is 10.0.12), and none for a launcher
/// shim, whose version is the shim's, not the program's.
fn plain_version(v: &str) -> Option<&str> {
    if v.to_ascii_lowercase().contains("shim") {
        return None;
    }
    v.split_whitespace().next()
}

/// A name and its version, as the agent reads it.
fn named(e: &FingerprintEntry) -> String {
    match e.version.as_deref().and_then(plain_version) {
        Some(v) => format!("{} {v}", e.name),
        None => e.name.clone(),
    }
}

/// The context block an agent is given.
pub fn describe(f: &Facts) -> String {
    let mut out = String::from(
        "The machine the plan will run on, as KeyJutsu found it. Write commands for these versions \
         and folders rather than assuming others.\n",
    );
    out.push_str(&format!("Windows: {}, build {}, {}\n", f.os, f.build, f.architecture));
    let shells: Vec<String> = f.shells.iter().map(named).collect();
    out.push_str(&format!(
        "Shells: {}\n",
        if shells.is_empty() { "none found".into() } else { shells.join(", ") }
    ));
    out.push_str(if f.elevated {
        "KeyJutsu is running as Administrator.\n"
    } else {
        "KeyJutsu is running without Administrator rights; a step that needs them must say so (privilege).\n"
    });
    if !f.folders.is_empty() {
        let folders: Vec<String> = f.folders.iter().map(|(name, at)| format!("{name} is {at}")).collect();
        out.push_str(&format!(
            "Folders: {}. In commands, $env:USERPROFILE is %USERPROFILE%.\n",
            folders.join("; ")
        ));
    }
    let (found, missing): (Vec<&FingerprintEntry>, Vec<&FingerprintEntry>) =
        f.tools.iter().partition(|t| t.path.is_some());
    if !found.is_empty() {
        let found: Vec<String> = found.iter().map(|t| named(t)).collect();
        out.push_str(&format!("On PATH: {}\n", found.join(", ")));
    }
    if !missing.is_empty() {
        let missing: Vec<&str> = missing.iter().map(|t| t.name.as_str()).collect();
        out.push_str(&format!("Not on PATH: {}\n", missing.join(", ")));
    }
    out
}

/// Where Desktop, Documents and Downloads are, from the `User Shell Folders`
/// key, which keeps them unexpanded (`%USERPROFILE%\OneDrive\Desktop`).
fn known_folders() -> Vec<(String, String)> {
    let output = keyjutsu_terminal::shell::command("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let profile = std::env::var("USERPROFILE").ok();
    parse_folders(&output, profile.as_deref())
}

fn parse_folders(reg_output: &str, profile: Option<&str>) -> Vec<(String, String)> {
    let value = |name: &str| -> Option<String> {
        reg_output.lines().find_map(|line| {
            let mut parts = line.split_whitespace();
            (parts.next() == Some(name)).then(|| {
                let _kind = parts.next();
                parts.collect::<Vec<_>>().join(" ")
            })
        })
    };
    [
        ("Desktop", "Desktop"),
        ("Documents", "Personal"),
        ("Downloads", "{374DE290-123F-4565-9164-39C4925E467B}"),
    ]
    .into_iter()
    .filter_map(|(name, key)| {
        let at = value(key)?;
        // A folder stored in full would name the user; say it as Windows
        // does when it keeps it relative.
        let at = match profile {
            Some(p) if at.to_ascii_lowercase().starts_with(&p.to_ascii_lowercase()) => {
                format!("%USERPROFILE%{}", &at[p.len()..])
            }
            _ => at,
        };
        Some((name.to_owned(), at))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const REG: &str = r"
HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders
    Desktop    REG_EXPAND_SZ    %USERPROFILE%\OneDrive\Desktop
    Personal    REG_EXPAND_SZ    C:\Users\someone\OneDrive - Contoso\Documents
    {374DE290-123F-4565-9164-39C4925E467B}    REG_EXPAND_SZ    %USERPROFILE%\Downloads
    My Music    REG_EXPAND_SZ    %USERPROFILE%\Music
";

    /// A Desktop in OneDrive is what a first plan got wrong; a folder stored
    /// in full must not carry the user's name to the agent.
    #[test]
    fn folders_are_where_windows_keeps_them_without_the_users_name() {
        let f = parse_folders(REG, Some(r"C:\Users\someone"));
        assert_eq!(
            f,
            [
                ("Desktop".to_owned(), r"%USERPROFILE%\OneDrive\Desktop".to_owned()),
                ("Documents".to_owned(), r"%USERPROFILE%\OneDrive - Contoso\Documents".to_owned()),
                ("Downloads".to_owned(), r"%USERPROFILE%\Downloads".to_owned()),
            ]
        );
        assert!(parse_folders("", None).is_empty());
    }

    fn entry(name: &str, path: Option<&str>, version: Option<&str>) -> FingerprintEntry {
        FingerprintEntry { name: name.into(), path: path.map(Into::into), version: version.map(Into::into) }
    }

    #[test]
    fn the_agent_is_told_versions_folders_and_what_is_missing() {
        let facts = Facts {
            os: "Windows 11 Pro 25H2".into(),
            build: "26200.6899".into(),
            architecture: "x64".into(),
            shells: vec![
                entry("pwsh", Some(r"C:\p\pwsh.exe"), Some("7.6.6")),
                entry("windows_powershell", Some(r"C:\w\powershell.exe"), Some("5.1.26100.6899")),
                entry("cmd", Some(r"C:\w\cmd.exe"), None),
            ],
            elevated: false,
            folders: vec![("Desktop".into(), r"%USERPROFILE%\OneDrive\Desktop".into())],
            tools: vec![
                entry("git", Some(r"C:\g\git.exe"), Some("2.47.1")),
                entry("dotnet", Some(r"C:\d\dotnet.exe"), Some("10.0.12 @Commit: 95017c711e6a")),
                entry("choco", Some(r"C:\c\choco.exe"), Some("0.12.1 - Chocolatey Shim: 1.0.0")),
                entry("docker", None, None),
            ],
        };
        let text = describe(&facts);
        for expected in [
            "Windows: Windows 11 Pro 25H2, build 26200.6899, x64",
            "Shells: pwsh 7.6.6, windows_powershell 5.1.26100.6899, cmd",
            "without Administrator rights",
            r"Desktop is %USERPROFILE%\OneDrive\Desktop",
            "On PATH: git 2.47.1, dotnet 10.0.12, choco\n",
            "Not on PATH: docker",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        assert!(!text.contains(r"C:\p\pwsh.exe"), "paths are not sent: {text}");
    }
}
