//! What this Windows account approved and wrote, recorded where only it can
//! vouch for it.
//!
//! A snapshot's hashes catch accidental edits, but they are unkeyed: someone
//! who can write the file can recompute them. So sealing a plan also puts a
//! record of its snapshot hash in the encrypted store (ADR 0016), and a
//! snapshot is only run or recovered if that record is there. Checkpoints
//! are treated the same way: every save records the SHA-256 of what was
//! written, and loading refuses a file that differs.
//!
//! The store's key is DPAPI-protected, so a record can be made only by this
//! Windows account on this machine. Another account, a copied disk or a file
//! handed over by someone else cannot produce one. A program running as the
//! same user can, as it can run commands directly; that is Windows' boundary,
//! not KeyJutsu's.

use std::path::Path;

use keyjutsu_plan::approval::ApprovedSnapshot;
use keyjutsu_plan::hash::sha256_hex;
use serde::{Deserialize, Serialize};

use crate::execute::Checkpoint;
use crate::store::Store;

const APPROVAL: &str = "approval";
const CHECKPOINT: &str = "checkpoint";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ApprovalRecord {
    snapshot_hash: String,
}

/// The hashes of the last two contents written, so a crash between
/// recording a save and completing it leaves a file that still matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CheckpointRecord {
    latest: String,
    previous: Option<String>,
}

fn approval_id(snapshot: &ApprovedSnapshot) -> String {
    let hash = snapshot.snapshot_hash();
    hash.strip_prefix("sha256:").unwrap_or(hash).to_ascii_lowercase()
}

/// The record for a checkpoint is found by its full path, as Windows compares
/// paths: without regard to case.
fn checkpoint_id(path: &Path) -> Result<String, String> {
    let full = std::path::absolute(path).map_err(|e| e.to_string())?;
    Ok(sha256_hex(full.to_string_lossy().to_lowercase().as_bytes()))
}

/// Record that this account approved `snapshot`. Called when a plan is sealed.
pub fn record_approval(store: &Store, snapshot: &ApprovedSnapshot) -> Result<(), String> {
    store.put(
        APPROVAL,
        &approval_id(snapshot),
        &ApprovalRecord { snapshot_hash: snapshot.snapshot_hash().to_owned() },
    )
}

/// Refuse a snapshot this account never approved on this machine, however
/// consistent its hashes are.
pub fn check_approval(store: &Store, snapshot: &ApprovedSnapshot) -> Result<(), String> {
    match store.get::<ApprovalRecord>(APPROVAL, &approval_id(snapshot))? {
        Some(r) if r.snapshot_hash == snapshot.snapshot_hash() => Ok(()),
        _ => Err(format!(
            "snapshot {} was not approved by this Windows account on this machine; \
             approve the plan here (`keyjutsu plan approve`) before running it",
            snapshot.snapshot_hash()
        )),
    }
}

/// Write `checkpoint` to `path`, recording what was written first.
pub fn save_checkpoint(store: &Store, checkpoint: &Checkpoint, path: &Path) -> Result<(), String> {
    let text = checkpoint.to_json();
    let id = checkpoint_id(path)?;
    let previous = store.get::<CheckpointRecord>(CHECKPOINT, &id).ok().flatten().map(|r| r.latest);
    store.put(CHECKPOINT, &id, &CheckpointRecord { latest: sha256_hex(text.as_bytes()), previous })?;
    Checkpoint::write(path, &text)
}

/// Read the checkpoint at `path`, refusing it unless KeyJutsu wrote exactly
/// this for this account.
pub fn load_checkpoint(store: &Store, path: &Path) -> Result<Checkpoint, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let written = sha256_hex(text.as_bytes());
    let record = store.get::<CheckpointRecord>(CHECKPOINT, &checkpoint_id(path)?)?;
    match record {
        Some(r) if r.latest == written || r.previous.as_deref() == Some(written.as_str()) => {
            Checkpoint::parse(&text)
        }
        Some(_) => Err("the checkpoint has been changed since KeyJutsu wrote it".into()),
        None => Err("KeyJutsu has no record of writing this checkpoint for this Windows account".into()),
    }
}
