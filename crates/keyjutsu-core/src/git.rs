//! Git-aware safety: tell KeyJutsu's changes from the operator's.
//!
//! Before a run, the repository a plan works in is recorded: branch, HEAD,
//! remotes, and every file that already differs from HEAD, with a hash of its
//! content and a copy of it. After the run it is recorded again. A file whose
//! content is exactly as it was is the operator's change, untouched; anything
//! else that differs is KeyJutsu's, and its diff is taken against what the
//! file was just before the run, not against HEAD, so the operator's earlier
//! edits never show up as KeyJutsu's.
//!
//! Everything here reads, except [`create_worktree`], which the operator asks
//! for. Nothing here commits, pushes or rewrites history: those are plan
//! steps, approved like any other.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use keyjutsu_plan::hash::sha256_hex;
use keyjutsu_validation::process::Finished;
use serde::{Deserialize, Serialize};

/// Files larger than this are hashed but not copied: their diff cannot be shown.
const MAX_COPY: u64 = 16 * 1024 * 1024;
/// Diffs longer than this are cut short in a report.
const MAX_DIFF: usize = 200 * 1024;

/// One file that differs from HEAD, as `git status` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "git/")]
pub struct Dirty {
    pub path: String,
    /// The two-letter porcelain status: ` M`, `M `, `??`, ` D`, `R `…
    pub status: String,
    /// Of the working file; `None` when it does not exist (deleted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "git/")]
pub struct RepoState {
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head: Option<String>,
    pub remotes: Vec<String>,
    pub dirty: Vec<Dirty>,
}

fn git(root: &Path, args: &[&str]) -> Result<Finished, String> {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).args(["-c", "core.quotepath=off", "--no-pager"]).args(args);
    keyjutsu_validation::process::run(c, "", Duration::from_secs(60)).map_err(|e| e.to_string())
}

fn text(out: &Finished) -> String {
    out.stdout.trim_end_matches(['\r', '\n']).to_owned()
}

/// The root of the repository containing `dir`, if it is in one.
pub fn find_root(dir: &Path) -> Option<PathBuf> {
    let out = git(dir, &["rev-parse", "--show-toplevel"]).ok()?;
    out.success.then(|| PathBuf::from(text(&out))).filter(|p| !p.as_os_str().is_empty())
}

/// Parse `git status --porcelain=v1 -z`. A rename or copy is followed by its
/// source path, which is skipped: the new path is what exists now.
pub fn parse_status(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut fields = raw.split('\0').filter(|f| !f.is_empty());
    while let Some(entry) = fields.next() {
        if entry.len() < 4 {
            continue;
        }
        let (status, path) = entry.split_at(2);
        let path = path[1..].to_owned();
        if status.contains('R') || status.contains('C') {
            fields.next();
        }
        out.push((status.to_owned(), path));
    }
    out
}

/// Record the repository at `root`. With `copies`, each changed file's
/// content is also kept there, named by its hash, so KeyJutsu's diff can be
/// taken against it later; with `store`, encrypted with its key (ADR 0018),
/// since a changed file the operator had not committed may be an `.env`.
pub fn record(
    root: &Path,
    copies: Option<&Path>,
    store: Option<&crate::store::Store>,
) -> Result<RepoState, String> {
    let status = git(root, &["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    if !status.success {
        return Err(format!("git status failed: {}", status.stderr.trim()));
    }
    let raw = status.stdout.clone();
    let branch =
        git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).ok().filter(|o| o.success).map(|o| text(&o));
    let head = git(root, &["rev-parse", "HEAD"]).ok().filter(|o| o.success).map(|o| text(&o));
    let remotes = git(root, &["remote"])
        .ok()
        .map(|o| text(&o).lines().map(str::to_owned).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default();
    if let Some(dir) = copies {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut dirty = Vec::new();
    for (status, path) in parse_status(&raw) {
        let file = root.join(&path);
        let sha256 = match std::fs::metadata(&file) {
            Ok(m) if m.is_file() => {
                let bytes = std::fs::read(&file).map_err(|e| format!("cannot read {path}: {e}"))?;
                let sha = sha256_hex(&bytes);
                if let Some(dir) = copies
                    && m.len() <= MAX_COPY
                {
                    let copy = dir.join(&sha);
                    if !copy.exists() {
                        let kept = match store {
                            Some(s) => s.seal(&sha, &bytes)?,
                            None => bytes.clone(),
                        };
                        std::fs::write(&copy, kept).map_err(|e| e.to_string())?;
                    }
                }
                Some(sha)
            }
            _ => None,
        };
        dirty.push(Dirty { path, status, sha256 });
    }
    Ok(RepoState { root: root.display().to_string(), branch, head, remotes, dirty })
}

/// One file KeyJutsu changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "git/")]
pub struct KeyJutsuChange {
    pub path: String,
    /// The operator had already changed this file before the run.
    pub was_already_changed: bool,
    /// What `git status` says now, `  ` if it is back to HEAD.
    pub status: String,
    /// KeyJutsu's diff alone: against the file as it was just before the run.
    pub diff: String,
}

/// What a run did to one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "git/")]
pub struct RepoReport {
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub branch: Option<String>,
    pub keyjutsu: Vec<KeyJutsuChange>,
    /// The operator's own earlier changes, exactly as they were.
    pub untouched: Vec<String>,
    /// HEAD before and after, when a step committed or switched branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub head_moved: Option<(String, String)>,
}

fn run_diff(root: &Path, args: &[&str]) -> String {
    match git(root, args) {
        // --no-index exits 1 when the files differ.
        Ok(o) => o.stdout,
        Err(e) => format!("(no diff: {e})"),
    }
}

/// KeyJutsu's diff for `path`: against the operator's copy if the file was
/// already changed, against the recorded HEAD if it was clean, against
/// nothing if it did not exist.
fn keyjutsu_diff(
    root: &Path,
    before: &RepoState,
    earlier: Option<&Dirty>,
    copies: &Path,
    store: Option<&crate::store::Store>,
    path: &str,
) -> String {
    let now = root.join(path);
    let fwd = |p: &Path| p.display().to_string().replace('\\', "/");
    let empty = copies.join("empty");
    let _ = std::fs::write(&empty, b"");
    // An encrypted copy is opened into a file of its own for git to read,
    // removed as soon as the diff is taken.
    let mut opened: Option<PathBuf> = None;
    let base: Option<PathBuf> = match earlier {
        Some(d) => match &d.sha256 {
            Some(sha) if copies.join(sha).exists() => {
                let copy = copies.join(sha);
                match std::fs::read(&copy) {
                    Ok(bytes) if crate::store::is_sealed(&bytes) => {
                        let Some(s) = store else {
                            return "(the earlier version is encrypted, and the store is not open)".into();
                        };
                        let plain = match s.unseal(sha, &bytes) {
                            Ok(p) => p,
                            Err(e) => return format!("({e})"),
                        };
                        let open = copies.join(format!("{sha}.open"));
                        if std::fs::write(&open, plain).is_err() {
                            return "(the earlier version could not be opened for the diff)".into();
                        }
                        opened = Some(open.clone());
                        Some(open)
                    }
                    _ => Some(copy),
                }
            }
            Some(_) => {
                return "(the earlier version was too large to keep, so the diff cannot be shown)".into();
            }
            None => Some(empty.clone()),
        },
        None => None,
    };
    let text = match base {
        Some(base) => {
            let target = if now.exists() { fwd(&now) } else { fwd(&empty) };
            let raw = run_diff(root, &["diff", "--no-index", "--no-color", "--", &fwd(&base), &target]);
            raw.replace(&fwd(&base).trim_start_matches('/').to_string(), path)
                .replace(&target.trim_start_matches('/').to_string(), path)
        }
        None => match &before.head {
            Some(head) if !now.exists() || tracked(root, path) => {
                run_diff(root, &["diff", "--no-color", head.as_str(), "--", path])
            }
            _ => {
                let raw =
                    run_diff(root, &["diff", "--no-index", "--no-color", "--", &fwd(&empty), &fwd(&now)]);
                raw.replace(&fwd(&empty).trim_start_matches('/').to_string(), "/dev/null")
                    .replace(&fwd(&now).trim_start_matches('/').to_string(), path)
            }
        },
    };
    if let Some(open) = opened {
        let _ = std::fs::remove_file(open);
    }
    if text.len() > MAX_DIFF {
        let mut cut = MAX_DIFF;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}\n… (diff cut at {} KiB)", &text[..cut], MAX_DIFF / 1024)
    } else {
        text
    }
}

fn tracked(root: &Path, path: &str) -> bool {
    git(root, &["ls-files", "--error-unmatch", "--", path]).is_ok_and(|o| o.success)
}

/// Whether `before` could have been written by [`record`]. `keyjutsu git
/// diff` reads it back from a file beside the snapshot, which anything
/// running as the user can edit: a HEAD of `--output=…` would become a git
/// option that writes a file, and a hash of `..\..\x` would diff a file
/// outside the copies. So HEAD and hashes must be hex, and paths must stay
/// inside the repository.
pub fn check_recorded(before: &RepoState) -> Result<(), String> {
    let hex = |s: &str, lens: &[usize]| lens.contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit());
    if let Some(head) = &before.head
        && !hex(head, &[40, 64])
    {
        return Err("the recorded HEAD is not a commit id; the record has been edited".into());
    }
    for d in &before.dirty {
        if d.sha256.as_deref().is_some_and(|s| !hex(s, &[64])) {
            return Err(format!("the recorded hash of {} is not a hash; the record has been edited", d.path));
        }
        let escapes = Path::new(&d.path).is_absolute()
            || d.path.contains(':')
            || d.path.split(['/', '\\']).any(|part| part == "..");
        if escapes {
            return Err(format!("{} is not inside the repository; the record has been edited", d.path));
        }
    }
    Ok(())
}

/// Compare the repository now with `before`, recorded just before the run.
pub fn report(
    before: &RepoState,
    copies: &Path,
    store: Option<&crate::store::Store>,
) -> Result<RepoReport, String> {
    check_recorded(before)?;
    let root = PathBuf::from(&before.root);
    let after = record(&root, None, None)?;
    let earlier: BTreeMap<&str, &Dirty> = before.dirty.iter().map(|d| (d.path.as_str(), d)).collect();
    let now: BTreeMap<&str, &Dirty> = after.dirty.iter().map(|d| (d.path.as_str(), d)).collect();
    let mut keyjutsu = Vec::new();
    let mut untouched = Vec::new();
    let paths: std::collections::BTreeSet<&str> = earlier.keys().chain(now.keys()).copied().collect();
    for path in paths {
        match (earlier.get(path), now.get(path)) {
            // Exactly as the operator left it.
            (Some(b), Some(a)) if b.sha256 == a.sha256 && b.status.trim() == a.status.trim() => {
                untouched.push(path.to_owned());
            }
            (b, a) => keyjutsu.push(KeyJutsuChange {
                path: path.to_owned(),
                was_already_changed: b.is_some(),
                status: a.map(|d| d.status.clone()).unwrap_or_else(|| "  ".into()),
                diff: keyjutsu_diff(&root, before, b.copied(), copies, store, path),
            }),
        }
    }
    let head_moved = match (&before.head, &after.head) {
        (Some(b), Some(a)) if b != a => Some((b.clone(), a.clone())),
        _ => None,
    };
    // Files a step committed are clean now but changed since the old HEAD.
    if let Some((from, to)) = &head_moved {
        let out = git(&root, &["diff", "--name-only", "-z", from.as_str(), to.as_str()])?;
        for path in out.stdout.split('\0').filter(|p| !p.is_empty()) {
            if !keyjutsu.iter().any(|k| k.path == path) {
                keyjutsu.push(KeyJutsuChange {
                    path: path.to_owned(),
                    was_already_changed: earlier.contains_key(path),
                    status: "committed".into(),
                    diff: run_diff(&root, &["diff", "--no-color", from.as_str(), to.as_str(), "--", path]),
                });
            }
        }
    }
    Ok(RepoReport { root: before.root.clone(), branch: after.branch, keyjutsu, untouched, head_moved })
}

/// The repositories a plan works in: the one the shell starts in, and any
/// named by a step's working directory.
pub fn repositories(start: &Path, plan: &keyjutsu_plan::model::Plan) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let dirs = std::iter::once(start.to_path_buf())
        .chain(plan.steps.iter().filter_map(|s| s.working_directory.as_ref().map(PathBuf::from)));
    for dir in dirs {
        if let Some(root) = find_root(&dir)
            && !roots.iter().any(|r| same(r, &root))
        {
            roots.push(root);
        }
    }
    roots
}

/// Whether two paths name the same folder, ignoring case and slash style.
/// The same folder, however it is written: case, slashes, or an 8.3 short
/// name (`RUNNER~1`) for a long one, which Windows gives out in places such
/// as `%TEMP%` while a shell reports the long form.
pub fn same_path(a: &Path, b: &Path) -> bool {
    same(a, b)
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if same(&x, &y)
        )
}

fn same(a: &Path, b: &Path) -> bool {
    a.display()
        .to_string()
        .replace('\\', "/")
        .eq_ignore_ascii_case(&b.display().to_string().replace('\\', "/"))
}

/// A temporary worktree on a new local branch, from HEAD, for a plan to run
/// in apart from the operator's working tree. The operator's
/// uncommitted changes are not in it.
pub fn create_worktree(root: &Path, branch: &str, dest: &Path) -> Result<PathBuf, String> {
    let out = git(root, &["worktree", "add", "-b", branch, &dest.display().to_string(), "HEAD"])?;
    if !out.success {
        return Err(format!("git worktree add failed: {}", out.stderr.trim()));
    }
    Ok(dest.to_path_buf())
}

/// A new local branch from HEAD, switched to in place. The operator's
/// uncommitted changes come along, as `git switch -c` always does; they stay
/// told apart by the record taken before the run.
pub fn create_branch(root: &Path, branch: &str) -> Result<(), String> {
    let out = git(root, &["switch", "-c", branch])?;
    if !out.success {
        return Err(format!("git switch -c failed: {}", out.stderr.trim()));
    }
    Ok(())
}

/// The branch name for a run's isolation: `keyjutsu/<plan>-<hash>`.
pub fn isolation_branch(plan_id: &str, snapshot_hash: &str) -> String {
    format!("keyjutsu/{plan_id}-{}", &snapshot_hash[..snapshot_hash.len().min(8)])
}

/// Whether any step names a folder inside `root`, which a worktree elsewhere
/// would not isolate.
pub fn steps_inside(root: &Path, plan: &keyjutsu_plan::model::Plan) -> Vec<String> {
    let r = root.display().to_string().replace('\\', "/").to_ascii_lowercase();
    plan.steps
        .iter()
        .filter(|s| {
            s.working_directory
                .as_ref()
                .is_some_and(|d| d.replace('\\', "/").to_ascii_lowercase().starts_with(&r))
        })
        .map(|s| s.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edited_git_record_is_refused_before_git_is_asked_anything() {
        let clean = || RepoState {
            root: r"C:\repo".into(),
            branch: None,
            head: Some("a".repeat(40)),
            remotes: vec![],
            dirty: vec![Dirty { path: "src/a.rs".into(), status: " M".into(), sha256: Some("b".repeat(64)) }],
        };
        assert_eq!(check_recorded(&clean()), Ok(()));
        let mut option = clean();
        option.head = Some(r"--output=C:\Users\Public\x".into());
        assert!(check_recorded(&option).is_err(), "a HEAD that git would read as an option");
        let mut hash = clean();
        hash.dirty[0].sha256 = Some(r"..\..\..\secret.txt".into());
        assert!(check_recorded(&hash).is_err(), "a hash that climbs out of the copies");
        for path in [r"..\outside.txt", r"C:\Windows\win.ini", "a/../../b", "C:x"] {
            let mut p = clean();
            p.dirty[0].path = path.into();
            assert!(check_recorded(&p).is_err(), "{path}");
        }
    }

    /// On a CI runner %TEMP% is a short name (`C:\Users\RUNNER~1\...`) while
    /// a shell reports the long one; the text differs, the folder doesn't.
    #[cfg(windows)]
    #[test]
    fn a_short_name_and_a_long_name_are_the_same_folder() {
        let long = std::env::temp_dir().join(format!("keyjutsu long folder name {}", std::process::id()));
        std::fs::create_dir_all(&long).expect("scratch folder");
        use std::os::windows::process::CommandExt;
        // raw_arg: cmd reads its own quotes, which Rust's escaping would break.
        let out = std::process::Command::new("cmd")
            .raw_arg(format!("/c for %I in (\"{}\") do @echo %~sI", long.display()))
            .output()
            .expect("cmd");
        let short = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        let differs = !same(&short, &long);
        assert!(same_path(&short, &long), "{} and {}", short.display(), long.display());
        assert!(!same_path(&long, &std::env::temp_dir()), "a different folder is not the same");
        let _ = std::fs::remove_dir(&long);
        if !differs {
            eprintln!("8.3 names are off on this volume; only the long form was compared");
        }
    }

    #[test]
    fn reads_porcelain_status_including_renames() {
        let raw = " M a.txt\0?? new file.txt\0R  b2.txt\0b.txt\0 D gone.txt\0";
        assert_eq!(
            parse_status(raw),
            [
                (" M".to_owned(), "a.txt".to_owned()),
                ("??".to_owned(), "new file.txt".to_owned()),
                ("R ".to_owned(), "b2.txt".to_owned()),
                (" D".to_owned(), "gone.txt".to_owned()),
            ]
        );
    }
}
