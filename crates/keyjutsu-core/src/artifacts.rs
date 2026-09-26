//! Staged artifacts: download, pin, verify, and run the staged copy.
//!
//! An artifact is downloaded before the plan is armed, into a store where
//! each copy is kept under its own SHA-256, with a record of where it came
//! from and when. At run time nothing is downloaded: just before a step, its
//! artifacts are hashed again and their paths handed to the step as
//! `$KJ_ARTIFACTS['name']`. A copy that no longer matches its pinned hash
//! stops the plan.

use std::path::{Path, PathBuf};
use std::time::Duration;

use keyjutsu_plan::hash::sha256_hex;
use keyjutsu_plan::model::{Artifact, Plan, Step};
use serde::{Deserialize, Serialize};

/// `%LOCALAPPDATA%\KeyJutsu\artifacts`, or a temporary folder if unset.
pub fn default_store() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("KeyJutsu")
        .join("artifacts")
}

/// Where a staged artifact came from, kept beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "artifacts/")]
pub struct StagedArtifact {
    pub name: String,
    pub source: String,
    pub sha256: String,
    pub path: String,
    pub size: u64,
    pub fetched_at: String,
    /// The plan pinned this hash before it was downloaded, rather than the
    /// download deciding it.
    pub was_pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub publisher: Option<String>,
}

/// A file name that is safe on disk and keeps the extension, which tools
/// such as `Expand-Archive` rely on.
fn file_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect();
    let clean = clean.trim_matches('.').to_owned();
    if clean.is_empty() { "artifact".into() } else { clean }
}

pub fn staged_path(store: &Path, sha256: &str, name: &str) -> PathBuf {
    store.join(sha256.to_ascii_lowercase()).join(file_name(name))
}

/// The staged copy of a pinned artifact, checked against its hash now.
pub fn verify(store: &Path, a: &Artifact) -> Result<PathBuf, String> {
    let Some(sha) = &a.sha256 else {
        return Err(format!("artifact `{}` is not pinned to a hash", a.name));
    };
    let path = staged_path(store, sha, &a.name);
    let bytes = std::fs::read(&path)
        .map_err(|_| format!("artifact `{}` is not staged: run `keyjutsu plan stage`", a.name))?;
    if !sha256_hex(&bytes).eq_ignore_ascii_case(sha) {
        return Err(format!("the staged copy of `{}` has changed since it was staged", a.name));
    }
    Ok(path)
}

fn download(source: &str, to: &Path) -> Result<(), String> {
    let ps = keyjutsu_terminal::shell::locate(keyjutsu_terminal::ShellKind::Pwsh)
        .or_else(|| keyjutsu_terminal::shell::locate(keyjutsu_terminal::ShellKind::WindowsPowershell))
        .ok_or("PowerShell is not installed")?;
    let script = format!(
        "$ErrorActionPreference = 'Stop'; $ProgressPreference = 'SilentlyContinue'; Invoke-WebRequest -Uri {} -OutFile {} -UseBasicParsing -TimeoutSec 300 -MaximumRedirection 5",
        crate::execute::ps_quote(source),
        crate::execute::ps_quote(&to.display().to_string())
    );
    let mut c = std::process::Command::new(ps);
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand"]);
    c.arg(keyjutsu_terminal::shell::encode_powershell_command(&script));
    let done =
        keyjutsu_validation::process::run(c, "", Duration::from_secs(330)).map_err(|e| e.to_string())?;
    if !done.success {
        let why = done.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("the download failed");
        return Err(format!("downloading {source}: {why}"));
    }
    Ok(())
}

/// Stage one artifact: download it (unless a verified copy is already
/// staged), check it against the pinned hash, keep it under its hash, and
/// record where it came from. A download that does not match the pin is
/// deleted, not kept.
pub fn stage(store: &Path, a: &Artifact, at: &str) -> Result<StagedArtifact, String> {
    let record = |sha: &str, path: &Path, size: u64, was_pinned: bool| StagedArtifact {
        name: a.name.clone(),
        source: a.source.clone(),
        sha256: sha.to_owned(),
        path: path.display().to_string(),
        size,
        fetched_at: at.to_owned(),
        was_pinned,
        version: a.version.clone(),
        publisher: a.publisher.clone(),
    };
    if let Ok(path) = verify(store, a) {
        let meta = path.with_file_name("provenance.json");
        if let Some(s) = std::fs::read_to_string(&meta).ok().and_then(|t| serde_json::from_str(&t).ok()) {
            return Ok(s);
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        return Ok(record(a.sha256.as_deref().unwrap_or_default(), &path, size, true));
    }
    std::fs::create_dir_all(store).map_err(|e| e.to_string())?;
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let incoming = store.join(format!(".incoming-{}-{nanos}", std::process::id()));
    let result = (|| {
        download(&a.source, &incoming)?;
        let bytes = std::fs::read(&incoming).map_err(|e| e.to_string())?;
        let sha = sha256_hex(&bytes);
        if let Some(pinned) = &a.sha256
            && !pinned.eq_ignore_ascii_case(&sha)
        {
            return Err(format!(
                "{} served content with sha256 {sha}, but the plan pins {pinned}; nothing was staged",
                a.source
            ));
        }
        let path = staged_path(store, &sha, &a.name);
        std::fs::create_dir_all(path.parent().unwrap_or(store)).map_err(|e| e.to_string())?;
        std::fs::rename(&incoming, &path).map_err(|e| e.to_string())?;
        let staged = record(&sha, &path, bytes.len() as u64, a.sha256.is_some());
        let meta = path.with_file_name("provenance.json");
        std::fs::write(meta, serde_json::to_string_pretty(&staged).unwrap_or_default())
            .map_err(|e| e.to_string())?;
        Ok(staged)
    })();
    let _ = std::fs::remove_file(&incoming);
    result
}

/// Every artifact in the plan, once each.
pub fn artifacts(plan: &Plan) -> Vec<&Artifact> {
    let mut out: Vec<&Artifact> = Vec::new();
    for a in plan.steps.iter().flat_map(|s| s.artifacts.iter()) {
        if !out.iter().any(|b| b.source == a.source && b.sha256 == a.sha256 && b.name == a.name) {
            out.push(a);
        }
    }
    out
}

/// The line that hands a step its artifacts, after checking each one again.
pub fn assignment(store: &Path, step: &Step) -> Result<Option<String>, String> {
    if step.artifacts.is_empty() {
        return Ok(None);
    }
    let mut pairs = Vec::new();
    for a in &step.artifacts {
        let path = verify(store, a)?;
        pairs.push(format!(
            "{} = {}",
            crate::execute::ps_quote(&a.name),
            crate::execute::ps_quote(&path.display().to_string())
        ));
    }
    Ok(Some(format!("$KJ_ARTIFACTS = @{{ {} }}", pairs.join("; "))))
}

/// The plan with each staged artifact's hash written in, for the operator
/// to review and approve: pinned before execution, never during it.
pub fn pin(plan: &Plan, staged: &[StagedArtifact]) -> Plan {
    let mut out = plan.clone();
    for step in &mut out.steps {
        for a in &mut step.artifacts {
            if a.sha256.is_none()
                && let Some(s) = staged.iter().find(|s| s.source == a.source && s.name == a.name)
            {
                a.sha256 = Some(s.sha256.clone());
            }
        }
    }
    out
}
