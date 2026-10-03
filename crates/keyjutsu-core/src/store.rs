//! KeyJutsu's local store, encrypted at rest (ADR 0016).
//!
//! Each record is a JSON document encrypted with AES-256-GCM under a random
//! 256-bit key. The key itself is kept only as DPAPI-protected bytes, so the
//! store can be read by this Windows user on this machine and nobody else.
//! A record's kind and id are bound into its encryption as associated data:
//! a record copied over another's file fails to decrypt rather than being
//! read as the other. Records are whole files written through a temporary
//! file and a rename, so a crash leaves the old record or the new one.
//!
//! Nothing secret goes in: credentials never reach the store, only the
//! plans, outcomes and history around them.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use serde::Serialize;
use serde::de::DeserializeOwned;

const MAGIC: &[u8; 4] = b"KJE1";
const COPY_MAGIC: &[u8; 4] = b"KJC1";

/// Whether `bytes` are a copy [`Store::seal`] made. Copies made before
/// ADR 0018, and the broker's own (kept where only Administrators can read),
/// are plain.
pub fn is_sealed(bytes: &[u8]) -> bool {
    bytes.starts_with(COPY_MAGIC)
}

/// `%LOCALAPPDATA%\KeyJutsu\store`, or `KEYJUTSU_STORE` when set (tests
/// use it to keep their sessions out of the operator's history).
pub fn default_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("KEYJUTSU_STORE") {
        return PathBuf::from(dir);
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("KeyJutsu")
        .join("store")
}

pub struct Store {
    root: PathBuf,
    cipher: Aes256Gcm,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the key.
        f.debug_struct("Store").field("root", &self.root).finish_non_exhaustive()
    }
}

/// The nonce stored after the four magic bytes; callers have checked that
/// there are at least sixteen.
fn nonce_at(bytes: &[u8]) -> Result<Nonce<<Aes256Gcm as AeadCore>::NonceSize>, String> {
    Nonce::try_from(&bytes[4..16]).map_err(|_| "a stored nonce has the wrong length".to_owned())
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl Store {
    /// Open the store at `root`, creating it and its key the first time.
    pub fn open(root: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let key_file = root.join("key.dpapi");
        let key = match std::fs::read(&key_file) {
            Ok(sealed) => crate::dpapi::unprotect(&sealed)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => create_key(&key_file)?,
            Err(e) => return Err(e.to_string()),
        };
        let key = Key::<Aes256Gcm>::try_from(key.as_slice())
            .map_err(|_| "the store key has the wrong length".to_owned())?;
        let cipher = Aes256Gcm::new(&key);
        Ok(Self { root: root.to_owned(), cipher })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, kind: &str, id: &str) -> Result<PathBuf, String> {
        if !valid_name(kind) || !valid_name(id) {
            return Err(format!("`{kind}/{id}` is not a valid record name"));
        }
        Ok(self.root.join(kind).join(format!("{id}.kje")))
    }

    fn aad(kind: &str, id: &str) -> Vec<u8> {
        format!("keyjutsu.store/1/{kind}/{id}").into_bytes()
    }

    pub fn put<T: Serialize>(&self, kind: &str, id: &str, value: &T) -> Result<(), String> {
        let path = self.path(kind, id)?;
        let plain = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
        let sealed = self
            .cipher
            .encrypt(&Nonce::from(nonce), Payload { msg: &plain, aad: &Self::aad(kind, id) })
            .map_err(|_| "encryption failed".to_owned())?;
        let mut out = Vec::with_capacity(4 + 12 + sealed.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        std::fs::create_dir_all(path.parent().unwrap_or(&self.root)).map_err(|e| e.to_string())?;
        write_atomic(&path, &out)
    }

    pub fn get<T: DeserializeOwned>(&self, kind: &str, id: &str) -> Result<Option<T>, String> {
        let path = self.path(kind, id)?;
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        if bytes.len() < 16 || &bytes[..4] != MAGIC {
            return Err(format!("{kind}/{id} is not a KeyJutsu record"));
        }
        let plain = self
            .cipher
            .decrypt(&nonce_at(&bytes)?, Payload { msg: &bytes[16..], aad: &Self::aad(kind, id) })
            .map_err(|_| {
                format!("{kind}/{id} could not be decrypted: it was altered or is not this record")
            })?;
        serde_json::from_slice(&plain).map(Some).map_err(|e| e.to_string())
    }

    /// The contents of a file KeyJutsu copies (a recovery backup, a Git
    /// copy), encrypted like a record and bound to `label`, what the copy is
    /// of, so one copy cannot be passed off as another (ADR 0018).
    pub fn seal(&self, label: &str, plain: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
        let sealed = self
            .cipher
            .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad: &Self::copy_aad(label) })
            .map_err(|_| "encryption failed".to_owned())?;
        let mut out = Vec::with_capacity(4 + 12 + sealed.len());
        out.extend_from_slice(COPY_MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    /// What [`Store::seal`] kept as `label`.
    pub fn unseal(&self, label: &str, sealed: &[u8]) -> Result<Vec<u8>, String> {
        if !is_sealed(sealed) || sealed.len() < 16 {
            return Err(format!("the copy of {label} is not a KeyJutsu copy"));
        }
        self.cipher
            .decrypt(&nonce_at(sealed)?, Payload { msg: &sealed[16..], aad: &Self::copy_aad(label) })
            .map_err(|_| {
                format!("the copy of {label} could not be decrypted: it was altered, or is not that copy")
            })
    }

    fn copy_aad(label: &str) -> Vec<u8> {
        format!("keyjutsu.copy/1/{label}").into_bytes()
    }

    /// The ids of every record of `kind`, sorted.
    pub fn list(&self, kind: &str) -> Result<Vec<String>, String> {
        if !valid_name(kind) {
            return Err(format!("`{kind}` is not a valid record kind"));
        }
        let mut ids: Vec<String> = match std::fs::read_dir(self.root.join(kind)) {
            Ok(entries) => entries
                .flatten()
                .filter_map(|e| {
                    e.file_name().to_str().and_then(|n| n.strip_suffix(".kje")).map(str::to_owned)
                })
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        ids.sort();
        Ok(ids)
    }

    /// Delete every record of `kind`. Returns how many there were.
    pub fn clear(&self, kind: &str) -> Result<usize, String> {
        let ids = self.list(kind)?;
        for id in &ids {
            std::fs::remove_file(self.path(kind, id)?).map_err(|e| e.to_string())?;
        }
        Ok(ids.len())
    }
}

/// Make the store's key, unless another process makes it first. The whole
/// key is written under a name of its own, then linked into place, which
/// fails if a key is already there; the loser uses the winner's key. Two
/// keys, the last one written winning, would leave records made under the
/// other unreadable.
fn create_key(key_file: &Path) -> Result<Vec<u8>, String> {
    let mut key = vec![0u8; 32];
    getrandom::fill(&mut key).map_err(|e| e.to_string())?;
    let sealed = crate::dpapi::protect(&key)?;
    let tmp = unique_tmp(key_file)?;
    std::fs::write(&tmp, &sealed).map_err(|e| e.to_string())?;
    let linked = std::fs::hard_link(&tmp, key_file);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(key),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            crate::dpapi::unprotect(&std::fs::read(key_file).map_err(|e| e.to_string())?)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// `path` with a random suffix, so writers never share a temporary file.
fn unique_tmp(path: &Path) -> Result<PathBuf, String> {
    let mut r = [0u8; 8];
    getrandom::fill(&mut r).map_err(|e| e.to_string())?;
    let suffix: String = r.iter().map(|b| format!("{b:02x}")).collect();
    Ok(path.with_extension(format!("{suffix}.tmp")))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = unique_tmp(path)?;
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// A short, sortable, unique id: `20260925-080000-a1b2`.
pub fn new_id(at: &str) -> String {
    let digits: String = at.chars().filter(char::is_ascii_digit).take(14).collect();
    let (date, time) = digits.split_at(digits.len().min(8));
    let mut r = [0u8; 2];
    let _ = getrandom::fill(&mut r);
    format!("{date}-{time}-{:02x}{:02x}", r[0], r[1])
}
