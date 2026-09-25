//! Windows DPAPI, for the one secret KeyJutsu keeps: the history store's key
//! (ADR 0016). Protected data can be unprotected only by the same Windows
//! user on the same machine, so a copied store is unreadable elsewhere.
//!
//! These are the only calls in this crate that need `unsafe`.

/// Mixed into the protection so another program using DPAPI for the same
/// user cannot unprotect KeyJutsu's key by accident.
const ENTROPY: &[u8] = b"KeyJutsu history store key v1";

#[cfg(windows)]
#[allow(unsafe_code)]
fn call(data: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    let len = u32::try_from(data.len()).map_err(|_| "too much data for DPAPI".to_owned())?;
    let input = CRYPT_INTEGER_BLOB { cbData: len, pbData: data.as_ptr().cast_mut() };
    let entropy = CRYPT_INTEGER_BLOB { cbData: ENTROPY.len() as u32, pbData: ENTROPY.as_ptr().cast_mut() };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: `input` and `entropy` point at live, correctly sized buffers
    // that DPAPI only reads (the functions take them as const pointers).
    // `output` is written by DPAPI with a buffer it allocates with
    // LocalAlloc, which is copied out and then freed exactly once below. No
    // prompt structure or description is passed, and UI is forbidden.
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 || output.pbData.is_null() {
        return Err(if protect {
            "Windows could not protect the store key".into()
        } else {
            "Windows could not unprotect the store key: it belongs to another user or machine".into()
        });
    }
    // SAFETY: on success DPAPI set `pbData` to a buffer of `cbData` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    // SAFETY: the buffer was allocated by DPAPI with LocalAlloc and is not
    // used after this.
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(bytes)
}

#[cfg(windows)]
pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    call(data, true)
}

#[cfg(windows)]
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    call(data, false)
}

#[cfg(not(windows))]
pub fn protect(_: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI is Windows-only".into())
}

#[cfg(not(windows))]
pub fn unprotect(_: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI is Windows-only".into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn protects_and_unprotects_for_this_user_only() {
        let secret = b"0123456789abcdef0123456789abcdef";
        let sealed = protect(secret).unwrap();
        assert!(!sealed.windows(secret.len()).any(|w| w == secret), "the key is not stored in the clear");
        assert_eq!(unprotect(&sealed).unwrap(), secret);
        let mut broken = sealed.clone();
        let last = broken.len() - 1;
        broken[last] ^= 1;
        assert!(unprotect(&broken).is_err());
    }
}
