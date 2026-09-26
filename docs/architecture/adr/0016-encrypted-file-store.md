# 0016: History is an encrypted file store under a DPAPI-protected key

Status: accepted.

## Context

KeyJutsu needs structured local persistence (SQLite, or an equivalent
robust embedded store), encrypted at rest with a random key protected by
Windows DPAPI. What is stored is small and read whole: a
session record (the sealed snapshot, checkpoint and outcome), a Technique
revision. There are tens or hundreds of them, not millions, and nothing
queries across their contents.

## Decision

- **One file per record**, `<store>\<kind>\<id>.kje`, written to a temporary
  file and renamed over the old one, so a crash leaves the old record or the
  new one, never half of either. The store is `%LOCALAPPDATA%\KeyJutsu\store`.
- **Each record is AES-256-GCM encrypted** (RustCrypto's `aes-gcm`) with a
  fresh 96-bit random nonce. The file is `KJE1`, the nonce, then the
  ciphertext and tag.
- **The record's kind and id are the associated data**, so a record copied
  over another record's file does not decrypt, rather than being read as that
  other record (`the_store_is_encrypted_and_records_cannot_be_swapped`, which
  fails with the binding removed).
- **The key is 32 random bytes kept only as DPAPI-protected data**
  (`key.dpapi`, current user, with an application-specific entropy value and
  no UI). A copy of the store is unreadable to another user or on another
  machine. The calls are the crate's only `unsafe` code, in `dpapi.rs`.
- **No SQLite.** `rusqlite` would add a C build and a second persistence
  model for data this small; a file per record, listed by directory, is
  robust enough and simple to reason about.
- **Revisions are never rewritten.** Saving a Technique revision that
  already exists is refused.

## Consequences

- There is no index: listing reads every record. That is fine at hundreds of
  records and would need revisiting at many thousands.
- Losing the Windows profile, or resetting its DPAPI master key, loses the
  history. That is the intended trade: nothing is recoverable by anyone else.
- `KEYJUTSU_STORE` moves the store; the tests use it so they never write to
  the operator's history.
- Staged artifacts are not encrypted: they are public downloads
  checked by hash. Run folders with snapshots and checkpoints are plain JSON
  so the CLI can use them; the encrypted copy of what matters is the session
  record.
