//! The store's bytes on disk, pinned. A record and a sealed copy written
//! under a fixed key, produced outside KeyJutsu by another AES-256-GCM
//! implementation (Python's `cryptography`), must still open: a change to
//! the cipher crate, or to how the store calls it, cannot then leave an
//! operator's history and Techniques unreadable without this failing.

#![allow(clippy::unwrap_used)]
#![cfg(windows)]

use keyjutsu_core::store::Store;

const KEY: [u8; 32] = {
    let mut k = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        k[i] = i as u8;
        i += 1;
    }
    k
};

/// `{"note":"written before the cipher crate changed"}` as record
/// `note/known-answer`, nonce a0..ab.
const RECORD: &str = "4b4a4531a0a1a2a3a4a5a6a7a8a9aaab9d3a124231ae20854012f5ba730ea5b050ce3c76fdc5274ce86643a61cc20569b704679cdd4327587fff6ca9671de69d6566e595b1bf320dce31207f8825c5b76ac5";

/// `a recovery copy\n` sealed as the copy of `C:/Users/Public/notes.txt`,
/// nonce b0..bb.
const COPY: &str = "4b4a4331b0b1b2b3b4b5b6b7b8b9babbf87528ce8fa2cd3a3581b7c1a22df1c8b0e8223754fffbaf9e05220224d34bfc";

fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
}

fn store_with_the_fixed_key(name: &str) -> Store {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("note")).unwrap();
    std::fs::write(root.join("key.dpapi"), keyjutsu_core::dpapi::protect(&KEY).unwrap()).unwrap();
    std::fs::write(root.join("note").join("known-answer.kje"), bytes(RECORD)).unwrap();
    Store::open(&root).unwrap()
}

#[test]
fn a_record_written_before_still_opens() {
    let store = store_with_the_fixed_key("store-format-record");
    let note: serde_json::Value = store.get("note", "known-answer").unwrap().unwrap();
    assert_eq!(note["note"], "written before the cipher crate changed");
}

#[test]
fn a_copy_sealed_before_still_unseals() {
    let store = store_with_the_fixed_key("store-format-copy");
    let plain = store.unseal("C:/Users/Public/notes.txt", &bytes(COPY)).unwrap();
    assert_eq!(plain, b"a recovery copy\n");
}

#[test]
fn the_record_is_still_bound_to_its_name() {
    let store = store_with_the_fixed_key("store-format-bound");
    let root = store.root().to_owned();
    std::fs::copy(root.join("note").join("known-answer.kje"), root.join("note").join("other.kje"))
        .unwrap();
    let err = store.get::<serde_json::Value>("note", "other").unwrap_err();
    assert!(err.contains("could not be decrypted"), "{err}");
}
