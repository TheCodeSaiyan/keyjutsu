//! What an agent is given to work from, and the manifest the operator sees
//! before anything is sent.
//!
//! Pasted text and individual files are read by KeyJutsu, redacted, and
//! placed in the prompt. A folder is not read: the agent is started in it and
//! investigates with its own tools, in its read-only mode, so KeyJutsu cannot
//! redact what it reads there. The manifest says so, and names files in the
//! folder that look like they hold secrets, so the operator can decide before
//! sending.
//!
//! Redaction is by pattern. It catches the common shapes (private keys, cloud
//! and platform tokens, `password=` style assignments) and says how many of
//! each it removed, never what they were. It cannot recognise a secret with
//! no recognisable shape, which is why invariant 2 is recorded as partial.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

/// The most text of any one item placed in a prompt.
pub const MAX_ITEM_CHARS: usize = 16_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextItem {
    Text { label: String, text: String },
    File(PathBuf),
    Folder(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "agent/")]
pub struct ManifestEntry {
    pub kind: String,
    pub label: String,
    /// Characters placed in the prompt; zero for a folder.
    pub chars_sent: usize,
    pub truncated: bool,
    /// Kinds of secret removed, with counts, e.g. `github token ×1`.
    pub redactions: Vec<String>,
    /// For a folder: files in it that look like they hold secrets.
    pub sensitive_files: Vec<String>,
}

/// The context as it will be sent, and the manifest that describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "agent/")]
pub struct PreparedContext {
    pub manifest: Vec<ManifestEntry>,
    /// The folder the agent will be started in, if any.
    pub working_directory: Option<String>,
    #[serde(skip)]
    #[ts(skip)]
    pub blocks: Vec<(String, String)>,
}

struct Pattern {
    kind: &'static str,
    re: Regex,
    /// Keep this capture group (a key name) and redact the rest.
    keep: Option<usize>,
}

#[allow(clippy::expect_used)] // Constant patterns; a bad one fails every test.
static PATTERNS: LazyLock<Vec<Pattern>> = LazyLock::new(|| {
    let p = |kind, re: &str, keep| Pattern { kind, re: Regex::new(re).expect("valid pattern"), keep };
    vec![
        p("private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----", None),
        p("AWS access key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", None),
        p("GitHub token", r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{40,})\b", None),
        p("Slack token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}", None),
        p("API key", r"\bsk-(?:ant-|proj-)?[A-Za-z0-9_-]{20,}", None),
        p("Google API key", r"\bAIza[0-9A-Za-z_-]{35}\b", None),
        p("JSON web token", r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}", None),
        p("bearer token", r"(?i)\b(bearer\s+)[A-Za-z0-9._~+/-]{16,}=*", Some(1)),
        p("storage account key", r"(?i)\b(AccountKey=)[A-Za-z0-9+/=]{20,}", Some(1)),
        p(
            "credential assignment",
            r#"(?i)\b((?:password|passwd|pwd|secret|client[_-]?secret|api[_-]?key|access[_-]?key|auth[_-]?token|token)\s*[:=]\s*)("[^"\r\n]*"|'[^'\r\n]*'|[^\s;,'"]+)"#,
            Some(1),
        ),
    ]
});

/// Remove recognisable secrets from `text`, returning the redacted text and
/// the kinds removed with their counts.
pub fn redact(text: &str) -> (String, Vec<String>) {
    let mut out = text.to_owned();
    let mut found = Vec::new();
    for pattern in PATTERNS.iter() {
        let mut count = 0;
        out = pattern
            .re
            .replace_all(&out, |c: &regex::Captures<'_>| {
                let whole = c.get(0).map(|m| m.as_str()).unwrap_or("");
                let kept = pattern.keep.and_then(|g| c.get(g)).map(|m| m.as_str()).unwrap_or("");
                // An earlier, more specific pattern already caught this value.
                if whole[kept.len()..].starts_with("[REDACTED") {
                    return whole.to_owned();
                }
                count += 1;
                format!("{kept}[REDACTED {}]", pattern.kind)
            })
            .into_owned();
        if count > 0 {
            found.push(format!("{} ×{count}", pattern.kind));
        }
    }
    (out, found)
}

/// File names that usually hold secrets.
fn looks_sensitive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".pfx")
        || lower.ends_with(".p12")
        || lower.ends_with(".key")
        || lower.starts_with("id_rsa")
        || lower.starts_with("id_ed25519")
        || lower.contains("credential")
        || lower.contains("secret")
        || lower == ".npmrc"
        || lower == ".pypirc"
        || lower == ".netrc"
}

/// Up to `limit` secret-looking files under `dir`, skipping build and
/// dependency folders. A bounded walk: this is a warning, not an audit.
fn sensitive_files(dir: &Path, limit: usize) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut visited = 0;
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            visited += 1;
            if found.len() >= limit || visited > 20_000 {
                return found;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if path.is_dir() {
                if !matches!(
                    name.as_str(),
                    ".git" | "node_modules" | "target" | "dist" | "bin" | "obj" | ".venv"
                ) {
                    stack.push(path);
                }
            } else if looks_sensitive(&name) {
                let rel = path.strip_prefix(dir).unwrap_or(&path);
                found.push(rel.display().to_string());
            }
        }
    }
    found.sort();
    found
}

fn cap(text: &str) -> (String, bool) {
    if text.chars().count() <= MAX_ITEM_CHARS {
        (text.to_owned(), false)
    } else {
        (text.chars().take(MAX_ITEM_CHARS).collect(), true)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContextError {
    #[error("cannot read {path}: {detail}")]
    Unreadable { path: String, detail: String },
    #[error("only one folder can be given; the agent is started in it")]
    SeveralFolders,
}

pub fn prepare(items: &[ContextItem]) -> Result<PreparedContext, ContextError> {
    let mut manifest = Vec::new();
    let mut blocks = Vec::new();
    let mut working_directory = None;
    for item in items {
        match item {
            ContextItem::Text { label, text } => {
                let (redacted, redactions) = redact(text);
                let (sent, truncated) = cap(&redacted);
                manifest.push(ManifestEntry {
                    kind: "text".into(),
                    label: label.clone(),
                    chars_sent: sent.chars().count(),
                    truncated,
                    redactions,
                    sensitive_files: Vec::new(),
                });
                blocks.push((label.clone(), sent));
            }
            ContextItem::File(path) => {
                let text = std::fs::read_to_string(path).map_err(|e| ContextError::Unreadable {
                    path: path.display().to_string(),
                    detail: e.to_string(),
                })?;
                let (redacted, redactions) = redact(&text);
                let (sent, truncated) = cap(&redacted);
                let label = path.display().to_string();
                manifest.push(ManifestEntry {
                    kind: "file".into(),
                    label: label.clone(),
                    chars_sent: sent.chars().count(),
                    truncated,
                    redactions,
                    sensitive_files: Vec::new(),
                });
                blocks.push((label, sent));
            }
            ContextItem::Folder(path) => {
                if working_directory.is_some() {
                    return Err(ContextError::SeveralFolders);
                }
                if !path.is_dir() {
                    return Err(ContextError::Unreadable {
                        path: path.display().to_string(),
                        detail: "not a folder".into(),
                    });
                }
                working_directory = Some(path.display().to_string());
                manifest.push(ManifestEntry {
                    kind: "folder".into(),
                    label: path.display().to_string(),
                    chars_sent: 0,
                    truncated: false,
                    redactions: Vec::new(),
                    sensitive_files: sensitive_files(path, 50),
                });
            }
        }
    }
    Ok(PreparedContext { manifest, working_directory, blocks })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_secret_shapes_are_removed_and_counted_but_never_echoed() {
        let text = concat!(
            "token = ghp_0123456789abcdefghijABCDEFGHIJ012345\n",
            "aws: AKIAABCDEFGHIJKLMNOP\n",
            "Authorization: Bearer abcdefghijklmnop0123456789\n",
            "DefaultEndpointsProtocol=https;AccountName=x;AccountKey=abcdefghijklmnopqrstuvwxyz0123456789ABCD==\n",
            "password=\"hunter2 with spaces\"\n",
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
            "ANTHROPIC_API_KEY=sk-ant-api03-abcdefghijklmnopqrstuvwxyz\n",
            "the word token on its own is fine\n",
        );
        let (out, found) = redact(text);
        for secret in [
            "ghp_0123456789",
            "AKIAABCDEFGHIJKLMNOP",
            "abcdefghijklmnop0123456789",
            "abcdefghijklmnopqrstuvwxyz0123456789ABCD",
            "hunter2",
            "b3BlbnNzaC1rZXktdjEAAAA",
            "sk-ant-api03",
        ] {
            assert!(!out.contains(secret), "{secret} survived:\n{out}");
        }
        assert!(out.contains("Bearer [REDACTED bearer token]"), "{out}");
        assert!(out.contains("token = [REDACTED GitHub token]\n"), "redacted once, not twice: {out}");
        assert!(out.contains("AccountKey=[REDACTED"), "the key name is kept: {out}");
        assert!(out.contains("the word token on its own is fine"));
        assert!(found.iter().any(|f| f.starts_with("private key")));
        assert!(found.iter().all(|f| !f.contains("hunter2")), "counts, not values");
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        let text = "Get-Service -Name docker\nThe token bucket refills at 5/s.\nkeyjutsu@example.test";
        assert_eq!(redact(text), (text.to_owned(), Vec::new()));
    }

    #[test]
    fn a_folder_is_not_read_but_its_secret_looking_files_are_named() {
        let dir = std::env::temp_dir().join(format!("keyjutsu-context-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules/x")).unwrap();
        std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
        std::fs::write(dir.join("config/server.pem"), "x").unwrap();
        std::fs::write(dir.join("node_modules/x/.env"), "ignored").unwrap();
        std::fs::write(dir.join("README.md"), "hello").unwrap();
        let prepared = prepare(&[ContextItem::Folder(dir.clone())]).unwrap();
        let entry = &prepared.manifest[0];
        assert_eq!(entry.chars_sent, 0);
        assert_eq!(
            entry.sensitive_files,
            vec![".env".to_owned(), format!("config{}server.pem", std::path::MAIN_SEPARATOR)]
        );
        assert!(prepared.blocks.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pasted_text_is_redacted_and_capped_before_it_is_sent() {
        let long = format!("password=abc123 {}", "x".repeat(MAX_ITEM_CHARS));
        let prepared = prepare(&[ContextItem::Text { label: "log".into(), text: long }]).unwrap();
        assert!(prepared.manifest[0].truncated);
        assert_eq!(prepared.manifest[0].chars_sent, MAX_ITEM_CHARS);
        assert!(!prepared.blocks[0].1.contains("abc123"));
    }
}
