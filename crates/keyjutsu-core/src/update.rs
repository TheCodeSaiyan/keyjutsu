//! Updates, when the operator asks for one (ADR 0019).
//!
//! KeyJutsu never looks for an update by itself. Asked, it reads the
//! release list from GitHub, picks the newest release on the operator's
//! channel, and compares it with the version running. Told to install it,
//! it downloads the installer and the release's `SHA256SUMS`, checks the
//! installer's hash against the list and its Authenticode signature, which
//! must verify and name KeyJutsu's publisher, and only then runs it. The
//! installer is the update: nothing new is signed, and Windows asks for
//! Administrator once, as for any install. Nothing starts while a run holds
//! the run lock, and the installer checks the lock again itself.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::plan::hash::sha256_hex;

/// Where releases are published.
pub const REPOSITORY: &str = "TheCodeSaiyan/keyjutsu";
/// The only publisher an installer may be signed by.
pub const PUBLISHER: &str = "TheCodeSaiyan Ltd";

/// Which releases the operator follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "update/")]
pub enum Channel {
    /// Releases only.
    #[default]
    Stable,
    /// Pre-releases too.
    Beta,
}

/// A release newer than the one running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "update/")]
pub struct Available {
    pub version: String,
    pub tag: String,
    pub prerelease: bool,
    /// What the release page says changed.
    pub notes: String,
    pub installer: String,
    #[serde(skip)]
    #[ts(skip)]
    installer_url: String,
    #[serde(skip)]
    #[ts(skip)]
    sums_url: String,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "update/")]
pub struct Checked {
    pub current: String,
    pub channel: Channel,
    /// `None` when the running version is the newest on the channel.
    pub available: Option<Available>,
}

/// The version running.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `1.2.3` or `1.2.3-beta.1`, with or without a leading `v`, as numbers and
/// pre-release identifiers. `None` for anything else.
fn parse(version: &str) -> Option<([u64; 3], Vec<String>)> {
    let v = version.trim().trim_start_matches('v');
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, p.split('.').map(str::to_owned).collect()),
        None => (v.split('+').next().unwrap_or(v), Vec::new()),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let n = [parts.next()??, parts.next()??, parts.next()??];
    if parts.next().is_some() {
        return None;
    }
    Some((n, pre))
}

/// Whether `candidate` is a later version than `current`, by semantic
/// versioning: a pre-release comes before its release.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some((a, ap)), Some((b, bp))) = (parse(candidate), parse(current)) else {
        return false;
    };
    if a != b {
        return a > b;
    }
    match (ap.is_empty(), bp.is_empty()) {
        (true, true) | (true, false) => !bp.is_empty(),
        (false, true) => false,
        (false, false) => {
            for (x, y) in ap.iter().zip(&bp) {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                    (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if order != std::cmp::Ordering::Equal {
                    return order == std::cmp::Ordering::Greater;
                }
            }
            ap.len() > bp.len()
        }
    }
}

/// The newest release on `channel` in GitHub's release list, if it is newer
/// than `current` and carries an installer and `SHA256SUMS`. Drafts never
/// count; a pre-release counts only on the Beta channel.
pub fn pick(releases: &serde_json::Value, channel: Channel, current: &str) -> Option<Available> {
    let mut best: Option<Available> = None;
    for r in releases.as_array()? {
        if r["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        let prerelease = r["prerelease"].as_bool().unwrap_or(false);
        if prerelease && channel == Channel::Stable {
            continue;
        }
        let Some(tag) = r["tag_name"].as_str() else { continue };
        let version = tag.trim_start_matches('v').to_owned();
        if parse(&version).is_none() || !is_newer(&version, current) {
            continue;
        }
        let asset = |f: &dyn Fn(&str) -> bool| {
            r["assets"].as_array().and_then(|a| {
                a.iter().find(|x| x["name"].as_str().is_some_and(f)).and_then(|x| {
                    Some((x["name"].as_str()?.to_owned(), x["browser_download_url"].as_str()?.to_owned()))
                })
            })
        };
        let installer = asset(&|n: &str| n.starts_with("KeyJutsu_") && n.ends_with("_x64-setup.exe"));
        let sums = asset(&|n: &str| n == "SHA256SUMS");
        let (Some((installer, installer_url)), Some((_, sums_url))) = (installer, sums) else { continue };
        if best.as_ref().is_none_or(|b| is_newer(&version, &b.version)) {
            best = Some(Available {
                version,
                tag: tag.to_owned(),
                prerelease,
                notes: what_changed(r["body"].as_str().unwrap_or_default()),
                installer,
                installer_url,
                sums_url,
            });
        }
    }
    best
}

/// What changed, from a release page: the notes written for that version,
/// without the install section every release page carries after them.
fn what_changed(body: &str) -> String {
    let body = body.replace("\r\n", "\n");
    let cut = body.find("\n## Install\n").unwrap_or(body.len());
    body[..cut].trim().to_owned()
}

fn powershell() -> Result<PathBuf, String> {
    keyjutsu_terminal::shell::locate(keyjutsu_terminal::ShellKind::Pwsh)
        .or_else(|| keyjutsu_terminal::shell::locate(keyjutsu_terminal::ShellKind::WindowsPowershell))
        .ok_or_else(|| "PowerShell is not installed".to_owned())
}

fn run_ps(script: &str, limit: Duration) -> Result<String, String> {
    let mut c = keyjutsu_terminal::shell::command(powershell()?);
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand"]);
    c.arg(keyjutsu_terminal::shell::encode_powershell_command(script));
    let done = keyjutsu_validation::process::run(c, "", limit).map_err(|e| e.to_string())?;
    if !done.success {
        let why = done.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("it failed").trim().to_owned();
        return Err(why);
    }
    Ok(done.stdout)
}

/// Ask GitHub for the newest release on `channel`. The one network request
/// KeyJutsu makes for updates, and only when the operator asks.
pub fn check(channel: Channel) -> Result<Checked, String> {
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12; \
         $r = Invoke-RestMethod -Uri 'https://api.github.com/repos/{REPOSITORY}/releases?per_page=30' -Headers @{{ 'User-Agent' = 'keyjutsu-update'; 'Accept' = 'application/vnd.github+json' }} -TimeoutSec 30; \
         ConvertTo-Json -InputObject @($r) -Depth 6 -Compress"
    );
    let text = run_ps(&script, Duration::from_secs(60)).map_err(|e| {
        format!(
            "KeyJutsu's releases could not be reached ({e}). While the repository is private, or with no network, there is nothing to update from"
        )
    })?;
    let releases: serde_json::Value = serde_json::from_str(text.trim()).map_err(|e| e.to_string())?;
    Ok(Checked {
        current: current_version().to_owned(),
        channel,
        available: pick(&releases, channel, current_version()),
    })
}

/// The hash `SHA256SUMS` lists for `name`, lower case.
pub fn listed_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// Whether a certificate subject (`CN=…, O=…`) names `PUBLISHER` as its
/// common name, exactly.
pub fn names_our_publisher(subject: &str) -> bool {
    subject.split(',').map(str::trim).any(|part| part.strip_prefix("CN=") == Some(PUBLISHER))
}

/// Check `installer` before anything runs it: its hash must be the one
/// `sums` lists for `name`, and its signature must verify and name
/// KeyJutsu's publisher.
pub fn verify(installer: &Path, sums: &str, name: &str) -> Result<(), String> {
    let listed = listed_hash(sums, name).ok_or_else(|| format!("SHA256SUMS does not list {name}"))?;
    let bytes = std::fs::read(installer).map_err(|e| e.to_string())?;
    let actual = sha256_hex(&bytes);
    if actual != listed {
        return Err(format!("{name} does not match SHA256SUMS (expected {listed}, got {actual})"));
    }
    let script = format!(
        "$s = Get-AuthenticodeSignature -LiteralPath {}; \
         [ordered]@{{ status = [string]$s.Status; subject = [string]$s.SignerCertificate.Subject }} | ConvertTo-Json -Compress",
        crate::execute::ps_quote(&installer.display().to_string())
    );
    let v: serde_json::Value =
        serde_json::from_str(run_ps(&script, Duration::from_secs(60))?.trim()).map_err(|e| e.to_string())?;
    let status = v["status"].as_str().unwrap_or_default();
    let subject = v["subject"].as_str().unwrap_or_default();
    if status != "Valid" {
        return Err(format!("{name}'s signature does not verify ({status}); an update must be signed"));
    }
    if !names_our_publisher(subject) {
        return Err(format!("{name} is signed by {subject}, not by {PUBLISHER}"));
    }
    Ok(())
}

/// Whether an update may start now: never while a run or recovery holds the
/// run lock.
pub fn free_to_install() -> Result<(), String> {
    crate::runlock::RunLock::take()
        .map(drop)
        .map_err(|why| format!("{why}. An update waits until nothing is changing the machine"))
}

/// Download `a`'s installer and `SHA256SUMS` into `dir` and check them. The
/// installer's path when both hold; nothing is kept when either does not.
pub fn download(a: &Available, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let installer = dir.join(&a.installer);
    let sums = dir.join("SHA256SUMS");
    let result = (|| {
        crate::artifacts::download(&a.installer_url, &installer)?;
        crate::artifacts::download(&a.sums_url, &sums)?;
        let listed = std::fs::read_to_string(&sums).map_err(|e| e.to_string())?;
        verify(&installer, &listed, &a.installer)
    })();
    match result {
        Ok(()) => Ok(installer),
        Err(e) => {
            let _ = std::fs::remove_file(&installer);
            let _ = std::fs::remove_file(&sums);
            Err(format!("{e}; nothing was installed"))
        }
    }
}

/// Start a checked installer, as double-clicking it would: Windows asks for
/// Administrator, and the installer asks its questions. Refused while a run
/// holds the run lock.
pub fn start(installer: &Path) -> Result<(), String> {
    free_to_install()?;
    let script =
        format!("Start-Process -FilePath {}", crate::execute::ps_quote(&installer.display().to_string()));
    run_ps(&script, Duration::from_secs(60)).map(drop)
}

/// Where downloads wait to be installed.
pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("keyjutsu-update")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_compare_as_semantic_versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.10.0", "0.9.0"), "numbers, not text");
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.2.0", "0.2.0-beta.1"), "a release comes after its pre-release");
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
        assert!(is_newer("0.2.0-beta.2", "0.2.0-beta.1"));
        assert!(is_newer("0.2.0-beta.10", "0.2.0-beta.9"));
        assert!(is_newer("0.2.0-rc.1", "0.2.0-beta.3"));
        assert!(!is_newer("nonsense", "0.1.0"));
        assert!(!is_newer("0.2", "0.1.0"));
    }

    fn release(tag: &str, prerelease: bool, draft: bool) -> serde_json::Value {
        let v = tag.trim_start_matches('v');
        json!({
            "tag_name": tag, "prerelease": prerelease, "draft": draft, "body": format!("notes for {tag}"),
            "assets": [
                {"name": format!("KeyJutsu_{v}_x64-setup.exe"), "browser_download_url": format!("https://example.test/{tag}/setup.exe")},
                {"name": "SHA256SUMS", "browser_download_url": format!("https://example.test/{tag}/SHA256SUMS")}
            ]
        })
    }

    #[test]
    fn the_newest_release_on_the_channel_is_offered_and_only_if_newer() {
        let list = json!([
            release("v0.3.0-beta.1", true, false),
            release("v0.4.0", false, true),
            release("v0.2.0", false, false),
            release("v0.1.0", false, false)
        ]);
        let stable = pick(&list, Channel::Stable, "0.1.0").unwrap();
        assert_eq!((stable.version.as_str(), stable.prerelease), ("0.2.0", false), "no drafts, no betas");
        assert_eq!(stable.installer, "KeyJutsu_0.2.0_x64-setup.exe");
        assert_eq!(stable.notes, "notes for v0.2.0");
        let beta = pick(&list, Channel::Beta, "0.1.0").unwrap();
        assert_eq!((beta.version.as_str(), beta.prerelease), ("0.3.0-beta.1", true));
        assert!(pick(&list, Channel::Stable, "0.2.0").is_none(), "up to date");
        assert!(pick(&json!([]), Channel::Stable, "0.1.0").is_none());
        // A release without an installer or its checksums is never offered.
        let mut bare = release("v0.5.0", false, false);
        bare["assets"] = json!([]);
        assert_eq!(
            pick(&json!([bare, release("v0.2.0", false, false)]), Channel::Stable, "0.1.0").unwrap().version,
            "0.2.0"
        );
    }

    #[test]
    fn an_update_shows_what_changed_not_how_to_install() {
        let page = "## Fixed\r\n\r\n- A thing.\r\n\r\n## Install\r\n\r\nIn PowerShell: irm … | iex\r\n\r\n## The small print\r\n\r\nSigned.";
        assert_eq!(what_changed(page), "## Fixed\n\n- A thing.");
        assert_eq!(what_changed("Only notes."), "Only notes.");
    }

    #[test]
    fn the_checksum_list_is_read_as_sha256sum_writes_it() {
        let h = "a".repeat(64);
        let sums = format!(
            "{h}  KeyJutsu_0.2.0_x64-setup.exe\n{}  KeyJutsu_0.2.0_x64-portable.zip\n",
            "b".repeat(64)
        );
        assert_eq!(listed_hash(&sums, "KeyJutsu_0.2.0_x64-setup.exe"), Some(h.clone()));
        assert_eq!(listed_hash(&format!("{}  *setup.exe", h.to_uppercase()), "setup.exe"), Some(h));
        assert_eq!(listed_hash(&sums, "other.exe"), None);
        assert_eq!(listed_hash("short  setup.exe", "setup.exe"), None);
    }

    #[test]
    fn only_our_publisher_is_accepted() {
        assert!(names_our_publisher("CN=TheCodeSaiyan Ltd, O=TheCodeSaiyan Ltd, L=London, C=GB"));
        assert!(!names_our_publisher("CN=Microsoft Windows, O=Microsoft Corporation, C=US"));
        assert!(!names_our_publisher("CN=TheCodeSaiyan Ltd Evil, O=Someone"));
        assert!(!names_our_publisher("O=TheCodeSaiyan Ltd, CN=Someone Else"));
        assert!(!names_our_publisher(""));
    }
}
