//! KeyJutsu's local store (§35), encrypted at rest (ADR 0016).
//!
//! Each record is a JSON document encrypted with AES-256-GCM under a random
//! 256-bit key. The key itself is kept only as DPAPI-protected bytes, so the
//! store can be read by this Windows user on this machine and nobody else.
//! A record's kind and id are bound into its encryption as associated data:
//! a record copied over another's file fails to decrypt rather than being
//! read as the other. Records are whole files written through a temporary
//! file and a rename, so a crash leaves the old record or the new one.
//!
//! Nothing secret goes in: credentials never reach the store (§25), only the
//! plans, outcomes and history around them.

use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use serde::Serialize;
use serde::de::DeserializeOwned;

const MAGIC: &[u8; 4] = b"KJE1";

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
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut key = vec![0u8; 32];
                getrandom::fill(&mut key).map_err(|e| e.to_string())?;
                let sealed = crate::dpapi::protect(&key)?;
                write_atomic(&key_file, &sealed)?;
                key
            }
            Err(e) => return Err(e.to_string()),
        };
        if key.len() != 32 {
            return Err("the store key has the wrong length".into());
        }
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
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
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: &plain, aad: &Self::aad(kind, id) })
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
            .decrypt(
                Nonce::from_slice(&bytes[4..16]),
                Payload { msg: &bytes[16..], aad: &Self::aad(kind, id) },
            )
            .map_err(|_| {
                format!("{kind}/{id} could not be decrypted: it was altered or is not this record")
            })?;
        serde_json::from_slice(&plain).map(Some).map_err(|e| e.to_string())
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

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// A short, sortable, unique id: `20260925-080000-a1b2`.
pub fn new_id(at: &str) -> String {
    let digits: String = at.chars().filter(char::is_ascii_digit).take(14).collect();
    let (date, time) = digits.split_at(digits.len().min(8));
    let mut r = [0u8; 2];
    let _ = getrandom::fill(&mut r);
    format!("{date}-{time}-{:02x}{:02x}", r[0], r[1])
}
