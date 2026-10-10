//! Which work profile applies to a plan.
//!
//! The default comes from the nearest `.keyjutsu/profile.toml` above the
//! shell's starting folder, then from this machine's default, and when
//! neither exists KeyJutsu asks on the first plan and keeps the answer as
//! the machine default. The operator can always choose by hand, and that is
//! recorded as an override.
//!
//! A folder must not be able to lower strictness without anyone noticing. So:
//!
//! - The starting folder is resolved to its real path first, following
//!   links, junctions and mapped drives, and both the search and any
//!   confirmation use that path. A path that cannot be resolved (an offline
//!   share, a disconnected drive, a folder the operator cannot open) makes
//!   the folder *broken*.
//! - A folder default less strict than the machine's (judged by ancestry:
//!   the folder's profile must extend the machine's, or be it) needs the
//!   operator's confirmation once for each real path and hash of the files,
//!   kept in the encrypted store. A changed file asks again.
//! - The hash covers only the files KeyJutsu loads, as exact bytes with CRLF
//!   turned into LF, so a checkout that converts line endings does not ask
//!   again but any other change does.
//! - Anything else in `.keyjutsu` breaks the folder, except a fixed list of
//!   files an operating system, Git or an editor leaves behind, which is
//!   ignored and kept out of the hash. A link or junction inside `.keyjutsu`,
//!   or a `.keyjutsu` that is one, breaks it too.
//! - A broken folder stops planning in that shell until the operator picks a
//!   profile by hand.
//!
//! Profiles on the machine live in `%LOCALAPPDATA%\KeyJutsu\profiles\*.toml`
//! and its default in `%LOCALAPPDATA%\KeyJutsu\work-profile.toml`. The
//! machine's files are trusted, so a change to one never asks about a folder
//! again.

use std::path::{Path, PathBuf};

use keyjutsu_validation::profile::{Catalogue, Origin, Profile};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::store::Store;

const FOLDER: &str = ".keyjutsu";
const FOLDER_DEFAULT: &str = "profile.toml";
const FOLDER_PROFILES: &str = "profiles";
const CONFIRMATIONS: &str = "profile-confirmations";

/// Where this machine keeps its profiles and its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Places {
    pub home: PathBuf,
}

impl Default for Places {
    /// `%LOCALAPPDATA%\KeyJutsu`, or `KEYJUTSU_PROFILE_HOME` when set (tests
    /// use it to keep the operator's own default out of their way).
    fn default() -> Self {
        let home = match std::env::var_os("KEYJUTSU_PROFILE_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
                .join("KeyJutsu"),
        };
        Self { home }
    }
}

impl Places {
    pub fn profiles(&self) -> PathBuf {
        self.home.join("profiles")
    }

    pub fn default_file(&self) -> PathBuf {
        self.home.join("work-profile.toml")
    }
}

/// `work-profile.toml` and `.keyjutsu/profile.toml`: which profile applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DefaultFile {
    default: String,
}

/// Why a profile applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The folder's `.keyjutsu/profile.toml` chose it.
    Folder { dir: String, hash: String },
    /// The folder chose a profile less strict than the machine's, and the
    /// operator confirmed it for these files.
    ConfirmedFolder { dir: String, hash: String },
    /// This machine's default.
    Machine,
    /// The operator chose it by hand, overriding what would have applied.
    HandPicked { instead_of: Option<String>, reason: String },
}

/// Where the starting folder was, as given and as resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub typed: PathBuf,
    /// The real path, without the `\\?\` prefix; `None` if it could not be resolved.
    pub real: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chosen {
    pub profile: Profile,
    pub why: Why,
    pub start: Start,
    /// Leftover files ignored in the folder's `.keyjutsu`, by relative path.
    pub leftovers: Vec<String>,
}

impl Chosen {
    /// What the plan records about why this profile applies.
    pub fn context(&self) -> keyjutsu_validation::ProfileContext {
        let folder_hash = match &self.why {
            Why::Folder { hash, .. } | Why::ConfirmedFolder { hash, .. } => Some(hash.clone()),
            _ => None,
        };
        keyjutsu_validation::ProfileContext {
            why: Some(describe(&self.why)),
            started_in: Some(self.start.typed.display().to_string()),
            real_path: self.start.real.clone(),
            folder_hash,
            leftovers: self.leftovers.clone(),
        }
    }
}

/// A folder's default that needs the operator's confirmation first.
#[derive(Debug, Clone, PartialEq)]
pub struct Unconfirmed {
    pub folder_profile: Profile,
    pub machine_profile: String,
    pub dir: String,
    pub hash: String,
    pub start: Start,
    pub leftovers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Chosen(Chosen),
    NeedsConfirmation(Unconfirmed),
    /// The folder, or the machine's profiles, could not be used: planning
    /// stops until a profile is picked by hand.
    Broken {
        start: Start,
        reason: String,
    },
    /// Nothing sets a default: ask, and keep the answer as the machine's.
    NoDefault {
        start: Start,
    },
}

/// What a confirmation keeps in the encrypted store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirmation {
    pub folder: String,
    pub hash: String,
    pub profile: String,
    pub machine_profile: String,
    pub at: String,
}

/// A folder's `.keyjutsu`, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FolderFiles {
    /// The folder holding `.keyjutsu`, as a real path.
    dir: String,
    default: String,
    /// Profile files: path relative to `.keyjutsu`, and text.
    profiles: Vec<(String, String)>,
    hash: String,
    leftovers: Vec<String>,
}

/// Decide which profile applies to plans made in a shell started in `start`.
pub fn decide(places: &Places, store: Option<&Store>, start: &Path) -> Decision {
    let typed = absolute(start);
    let real = real_path(&typed);
    let start_info = Start { typed: typed.clone(), real: real.as_ref().ok().cloned() };
    let broken = |reason: String| Decision::Broken { start: start_info.clone(), reason };
    let real = match real {
        Ok(r) => r,
        Err(e) => return broken(format!("the starting folder {} cannot be resolved: {e}", typed.display())),
    };
    let mut catalogue = match machine_catalogue(places) {
        Ok(c) => c,
        Err(e) => return broken(e),
    };
    let machine_default = match read_default(&places.default_file()) {
        Ok(d) => d,
        Err(e) => return broken(e),
    };
    let folder = match find_folder(Path::new(&real)) {
        Ok(f) => f,
        Err(e) => return broken(e),
    };
    let Some(folder) = folder else {
        return match machine_default {
            None => Decision::NoDefault { start: start_info },
            Some(name) => match catalogue.resolve(&name) {
                Ok(profile) => Decision::Chosen(Chosen {
                    profile,
                    why: Why::Machine,
                    start: start_info,
                    leftovers: Vec::new(),
                }),
                Err(e) => broken(format!("this machine's default profile: {e}")),
            },
        };
    };
    for (rel, text) in &folder.profiles {
        let origin = Origin::Folder(Path::new(&folder.dir).join(FOLDER).join(rel));
        if let Err(e) = catalogue.add(origin, text) {
            return broken(e.to_string());
        }
    }
    let profile = match catalogue.resolve(&folder.default) {
        Ok(p) => p,
        Err(e) => {
            return broken(format!(
                "{}: {e}",
                Path::new(&folder.dir).join(FOLDER).join(FOLDER_DEFAULT).display()
            ));
        }
    };
    let lower = match &machine_default {
        Some(m) => !profile.at_least_as_strict_as(m),
        None => false,
    };
    let (dir, hash, leftovers) = (folder.dir.clone(), folder.hash.clone(), folder.leftovers.clone());
    if !lower {
        return Decision::Chosen(Chosen {
            profile,
            why: Why::Folder { dir, hash },
            start: start_info,
            leftovers,
        });
    }
    let machine_profile = machine_default.unwrap_or_default();
    match store.map(|s| s.get::<Confirmation>(CONFIRMATIONS, &confirmation_id(&dir, &hash))) {
        Some(Ok(Some(c))) if c.folder == dir && c.hash == hash && c.profile == profile.name() => {
            Decision::Chosen(Chosen {
                profile,
                why: Why::ConfirmedFolder { dir, hash },
                start: start_info,
                leftovers,
            })
        }
        Some(Err(e)) => broken(format!("the folder's earlier confirmation cannot be read: {e}")),
        _ => Decision::NeedsConfirmation(Unconfirmed {
            folder_profile: profile,
            machine_profile,
            dir,
            hash,
            start: start_info,
            leftovers,
        }),
    }
}

/// Keep the operator's confirmation of a folder's less strict default.
pub fn confirm(store: &Store, u: &Unconfirmed, at: &str) -> Result<Chosen, String> {
    let record = Confirmation {
        folder: u.dir.clone(),
        hash: u.hash.clone(),
        profile: u.folder_profile.name().to_owned(),
        machine_profile: u.machine_profile.clone(),
        at: at.to_owned(),
    };
    store.put(CONFIRMATIONS, &confirmation_id(&u.dir, &u.hash), &record)?;
    Ok(Chosen {
        profile: u.folder_profile.clone(),
        why: Why::ConfirmedFolder { dir: u.dir.clone(), hash: u.hash.clone() },
        start: u.start.clone(),
        leftovers: u.leftovers.clone(),
    })
}

/// Every profile the operator can pick by hand in a shell started in
/// `start`: the built-ins, the machine's, and the folder's when the folder
/// can be used. A broken folder's profiles are left out, not trusted.
pub fn catalogue(places: &Places, start: &Path) -> Result<Catalogue, String> {
    let mut catalogue = machine_catalogue(places)?;
    if let Ok(real) = real_path(&absolute(start))
        && let Ok(Some(folder)) = find_folder(Path::new(&real))
    {
        let mut with_folder = catalogue.clone();
        let ok = folder.profiles.iter().all(|(rel, text)| {
            with_folder.add(Origin::Folder(Path::new(&folder.dir).join(FOLDER).join(rel)), text).is_ok()
        });
        if ok {
            catalogue = with_folder;
        }
    }
    Ok(catalogue)
}

/// Pick `name` by hand, overriding what `decision` said.
pub fn hand_pick(places: &Places, decision: &Decision, name: &str) -> Result<Chosen, String> {
    let (start, instead_of, reason) = match decision {
        Decision::Chosen(c) if c.profile.name() == name => return Ok(c.clone()),
        Decision::Chosen(c) => {
            (c.start.clone(), Some(c.profile.name().to_owned()), "chosen by hand".to_owned())
        }
        Decision::NeedsConfirmation(u) => (
            u.start.clone(),
            Some(u.machine_profile.clone()),
            format!("chosen by hand instead of confirming the folder's {}", u.folder_profile.name()),
        ),
        Decision::Broken { start, reason } => (start.clone(), None, format!("chosen by hand: {reason}")),
        Decision::NoDefault { start } => {
            (start.clone(), None, "chosen by hand; no default was set".to_owned())
        }
    };
    let profile = catalogue(places, &start.typed)?.resolve(name).map_err(|e| e.to_string())?;
    Ok(Chosen { profile, why: Why::HandPicked { instead_of, reason }, start, leftovers: Vec::new() })
}

/// The machine's default profile, if one is set.
pub fn machine_default(places: &Places) -> Result<Option<String>, String> {
    read_default(&places.default_file())
}

/// Make `name` the machine's default, after checking it resolves.
pub fn set_machine_default(places: &Places, name: &str) -> Result<(), String> {
    machine_catalogue(places)?.resolve(name).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&places.home).map_err(|e| e.to_string())?;
    let text = format!(
        "# KeyJutsu's default work profile on this machine. `keyjutsu profile default NAME` changes it.\n{}",
        toml::to_string(&DefaultFile { default: name.to_owned() }).map_err(|e| e.to_string())?
    );
    std::fs::write(places.default_file(), text).map_err(|e| e.to_string())
}

fn machine_catalogue(places: &Places) -> Result<Catalogue, String> {
    let mut catalogue = Catalogue::built_in();
    let dir = places.profiles();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(catalogue),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")))
        .collect();
    files.sort();
    for f in files {
        let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        catalogue.add(Origin::Machine(f), &text).map_err(|e| e.to_string())?;
    }
    Ok(catalogue)
}

fn read_default(file: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => toml::from_str::<DefaultFile>(&text)
            .map(|d| Some(d.default))
            .map_err(|e| format!("{}: {}", file.display(), e.message())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_owned()
    } else {
        std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_owned())
    }
}

/// The real path of a folder: links, junctions and mapped drives followed,
/// without the `\\?\` prefix. A mapped drive comes back as its UNC path.
pub fn real_path(p: &Path) -> Result<String, String> {
    let real = std::fs::canonicalize(p).map_err(|e| e.to_string())?;
    if !real.is_dir() {
        return Err("it is not a folder".into());
    }
    let s = real.display().to_string();
    Ok(match s.strip_prefix(r"\\?\UNC\") {
        Some(unc) => format!(r"\\{unc}"),
        None => s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s),
    })
}

/// The nearest folder at or above `real` whose `.keyjutsu` holds a
/// `profile.toml`, read and checked. Walking up the real path, not the typed
/// one, is what makes a junctioned folder find the same `.keyjutsu`.
fn find_folder(real: &Path) -> Result<Option<FolderFiles>, String> {
    for dir in real.ancestors() {
        let kj = dir.join(FOLDER);
        let Ok(meta) = std::fs::symlink_metadata(&kj) else { continue };
        if meta.file_type().is_symlink() {
            return Err(format!("{} is a link or junction, which KeyJutsu does not follow", kj.display()));
        }
        if !meta.is_dir() || !kj.join(FOLDER_DEFAULT).exists() {
            continue;
        }
        return read_folder(dir, &kj).map(Some);
    }
    Ok(None)
}

/// Files an operating system, Git or an editor leaves in a folder: ignored
/// wherever they are in `.keyjutsu`, and kept out of the hash.
fn leftover(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["desktop.ini", "thumbs.db", ".ds_store", ".gitkeep", "4913"].contains(&n.as_str())
        || [".swp", ".swo", ".swx", "~", ".bak", ".orig"].iter().any(|s| n.ends_with(s))
        || n.starts_with("~$")
}

fn read_folder(dir: &Path, kj: &Path) -> Result<FolderFiles, String> {
    let fail = |what: String| Err(format!("{}: {what}", kj.display()));
    let mut loaded: Vec<(String, Vec<u8>)> = Vec::new();
    let mut leftovers = Vec::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(kj.to_owned(), String::new())];
    while let Some((at, rel)) = stack.pop() {
        let entries = std::fs::read_dir(&at).map_err(|e| format!("{}: {e}", at.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let rel_path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let kind = meta.file_type();
            if kind.is_symlink() {
                return fail(format!("{rel_path} is a link or junction, which KeyJutsu does not follow"));
            }
            if kind.is_file() && leftover(&name) {
                leftovers.push(rel_path);
                continue;
            }
            let is_default = rel.is_empty() && name.eq_ignore_ascii_case(FOLDER_DEFAULT);
            let is_profile = rel.eq_ignore_ascii_case(FOLDER_PROFILES)
                && Path::new(&name).extension().is_some_and(|x| x.eq_ignore_ascii_case("toml"));
            if kind.is_file() && (is_default || is_profile) {
                let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                loaded.push((rel_path.to_ascii_lowercase(), bytes));
            } else if kind.is_dir() && rel.is_empty() && name.eq_ignore_ascii_case(FOLDER_PROFILES) {
                stack.push((path, rel_path));
            } else {
                return fail(format!(
                    "{rel_path} is not something KeyJutsu reads; only profile.toml and profiles/*.toml belong here"
                ));
            }
        }
    }
    loaded.sort();
    leftovers.sort();
    let mut digest = Sha256::new();
    let mut default = None;
    let mut profiles = Vec::new();
    for (rel, bytes) in &loaded {
        let normal = lf(bytes);
        digest.update((rel.len() as u64).to_le_bytes());
        digest.update(rel.as_bytes());
        digest.update((normal.len() as u64).to_le_bytes());
        digest.update(&normal);
        let text = String::from_utf8(normal).map_err(|_| format!("{}/{rel} is not UTF-8", kj.display()))?;
        if rel == FOLDER_DEFAULT {
            let d: DefaultFile =
                toml::from_str(&text).map_err(|e| format!("{}/{rel}: {}", kj.display(), e.message()))?;
            default = Some(d.default);
        } else {
            profiles.push((rel.clone(), text));
        }
    }
    let hash = digest.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok(FolderFiles {
        dir: dir.display().to_string(),
        default: default.ok_or_else(|| format!("{} has no {FOLDER_DEFAULT}", kj.display()))?,
        profiles,
        hash,
        leftovers,
    })
}

/// CRLF as LF, and nothing else changed.
fn lf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// One record per real folder and hash. The folder is compared without
/// regard to case, as Windows does.
fn confirmation_id(dir: &str, hash: &str) -> String {
    let digest = Sha256::digest(format!("{}\n{hash}", dir.to_lowercase()).as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// How the profile stands for a shell, as the desktop app shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "workspace/")]
pub enum ProfileState {
    /// A profile applies, and plans can be made.
    Chosen,
    /// The folder asks for less than the machine; confirm it or pick another.
    NeedsConfirmation,
    /// The folder cannot be used; pick a profile by hand.
    Broken,
    /// Nothing is set; choose this machine's default.
    NoDefault,
}

/// One profile the operator can choose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "workspace/")]
pub struct ProfileChoice {
    pub name: String,
    pub title: String,
    pub summary: Option<String>,
    pub extends: Option<String>,
}

/// Everything the desktop app shows about the work profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export, export_to = "workspace/")]
pub struct ProfileView {
    pub state: ProfileState,
    /// The profile that applies, when one does.
    pub name: Option<String>,
    pub title: Option<String>,
    /// Why it applies, or what stops one applying, in a sentence.
    pub why: String,
    /// The folder's less strict profile awaiting confirmation, by title.
    pub folder_profile: Option<String>,
    pub machine_default: Option<String>,
    pub hand_picked: bool,
    pub started_in: String,
    /// The real path, when it differs from where the shell started.
    pub real_path: Option<String>,
    pub leftovers: Vec<String>,
    pub choices: Vec<ProfileChoice>,
}

/// A profile picked by hand, and what it was picked against: when that
/// changes (the folder's files, the machine's default), the pick no longer
/// stands and the operator is asked again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pick {
    name: String,
    basis: String,
}

/// The work profile for one shell: where it started, and any profile picked
/// by hand in it. A pick lasts as long as the shell, so a shell restored
/// after a restart, being a new one, asks again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellProfile {
    start: PathBuf,
    pick: Option<Pick>,
}

/// What a decision rested on, for telling whether a pick still stands.
fn basis(d: &Decision) -> String {
    match d {
        Decision::Chosen(c) => match &c.why {
            Why::Folder { hash, .. } | Why::ConfirmedFolder { hash, .. } => {
                format!("folder {hash} {}", c.profile.name())
            }
            _ => format!("chosen {}", c.profile.name()),
        },
        Decision::NeedsConfirmation(u) => format!("confirm {} {}", u.hash, u.machine_profile),
        Decision::Broken { reason, .. } => format!("broken {reason}"),
        Decision::NoDefault { .. } => "none".into(),
    }
}

impl ShellProfile {
    pub fn new(start: PathBuf) -> Self {
        Self { start, pick: None }
    }

    /// Decide again, as before every plan, keeping a hand pick only while
    /// what it was picked against is unchanged.
    pub fn current(&mut self, places: &Places, store: Option<&Store>) -> (Decision, Option<Chosen>) {
        let decision = decide(places, store, &self.start);
        if let Some(pick) = &self.pick {
            if pick.basis == basis(&decision)
                && let Ok(c) = hand_pick(places, &decision, &pick.name)
            {
                return (decision, Some(c));
            }
            self.pick = None;
        }
        let chosen = match &decision {
            Decision::Chosen(c) => Some(c.clone()),
            _ => None,
        };
        (decision, chosen)
    }

    /// Pick `name` by hand for this shell.
    pub fn pick(&mut self, places: &Places, store: Option<&Store>, name: &str) -> Result<Chosen, String> {
        let decision = decide(places, store, &self.start);
        let chosen = hand_pick(places, &decision, name)?;
        self.pick = Some(Pick { name: name.to_owned(), basis: basis(&decision) });
        Ok(chosen)
    }

    /// Drop a hand pick, going back to what the folder or machine says.
    pub fn clear_pick(&mut self) {
        self.pick = None;
    }

    pub fn view(&mut self, places: &Places, store: Option<&Store>) -> ProfileView {
        let (decision, chosen) = self.current(places, store);
        let machine_default = machine_default(places).ok().flatten();
        let choices = catalogue(places, &self.start)
            .map(|c| {
                c.names()
                    .into_iter()
                    .filter_map(|(name, title)| {
                        let p = c.resolve(&name).ok()?;
                        Some(ProfileChoice {
                            summary: p.chain[0].summary.clone(),
                            extends: p.chain.get(1).map(|l| l.name.clone()),
                            name,
                            title,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let start = match &decision {
            Decision::Chosen(c) => c.start.clone(),
            Decision::NeedsConfirmation(u) => u.start.clone(),
            Decision::Broken { start, .. } | Decision::NoDefault { start } => start.clone(),
        };
        let started_in = start.typed.display().to_string();
        let real_path = start.real.filter(|r| Path::new(r) != start.typed);
        let base = ProfileView {
            state: ProfileState::Chosen,
            name: None,
            title: None,
            why: String::new(),
            folder_profile: None,
            machine_default,
            hand_picked: false,
            started_in,
            real_path,
            leftovers: Vec::new(),
            choices,
        };
        if let Some(c) = chosen {
            return ProfileView {
                name: Some(c.profile.name().to_owned()),
                title: Some(c.profile.title().to_owned()),
                why: describe(&c.why),
                hand_picked: matches!(c.why, Why::HandPicked { .. }),
                leftovers: c.leftovers,
                ..base
            };
        }
        match decision {
            Decision::NeedsConfirmation(u) => ProfileView {
                state: ProfileState::NeedsConfirmation,
                why: format!(
                    "{} chooses {}, which asks for less than this machine's {}.",
                    u.dir,
                    u.folder_profile.title(),
                    u.machine_profile
                ),
                folder_profile: Some(u.folder_profile.title().to_owned()),
                ..base
            },
            Decision::Broken { reason, .. } => {
                ProfileView { state: ProfileState::Broken, why: sentence(&reason), ..base }
            }
            Decision::NoDefault { .. } | Decision::Chosen(_) => ProfileView {
                state: ProfileState::NoDefault,
                why: "No work profile is set for this machine yet.".into(),
                ..base
            },
        }
    }
}

/// `text` as a sentence: a capital first letter and a full stop.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    let rest: String = chars.collect();
    let end = if rest.ends_with('.') { "" } else { "." };
    format!("{first}{rest}{end}")
}

/// Why a profile applies, in a sentence.
pub fn describe(why: &Why) -> String {
    match why {
        Why::Folder { dir, .. } => format!("Set by {dir}."),
        Why::ConfirmedFolder { dir, .. } => format!("Set by {dir}, which you confirmed."),
        Why::Machine => "This machine's default.".into(),
        Why::HandPicked { instead_of: Some(other), reason } => {
            let reason = reason
                .strip_prefix("chosen by hand")
                .unwrap_or(reason)
                .trim_start_matches([',', ':', ';', ' ']);
            if reason.is_empty() {
                format!("Chosen by hand, instead of {other}.")
            } else {
                format!("Chosen by hand, instead of {other}: {reason}.")
            }
        }
        Why::HandPicked { instead_of: None, reason } => {
            let reason = reason
                .strip_prefix("chosen by hand")
                .unwrap_or(reason)
                .trim_start_matches([',', ':', ';', ' ']);
            format!("Chosen by hand: {reason}.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leftovers_are_a_fixed_list() {
        for n in ["desktop.ini", "Thumbs.db", ".DS_Store", ".gitkeep", "profile.toml.swp", "x~", "~$doc"] {
            assert!(leftover(n), "{n}");
        }
        for n in ["notes.txt", "profile.toml", "readme.md"] {
            assert!(!leftover(n), "{n}");
        }
    }

    #[test]
    fn only_crlf_changes() {
        assert_eq!(lf(b"a\r\nb\rc\n"), b"a\nb\rc\n");
    }
}
