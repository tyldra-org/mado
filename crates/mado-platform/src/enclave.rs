//! The Secure Enclave key that wraps the vault key (spec 11 §2.1), through
//! CryptoKit in `swift/enclave.swift`. The key blob is usable only by this
//! Mac's Secure Enclave, and is bound to the Mac rather than to the app's
//! code signature, so a self-signed update uses it with no new prompt.

use std::ffi::CString;

use crate::PlatformError;

unsafe extern "C" {
    fn mado_enclave_create(
        presence: bool,
        key: *mut u8,
        key_cap: isize,
        key_len: *mut isize,
        public: *mut u8,
        public_cap: isize,
    ) -> isize;
    fn mado_enclave_agree(
        key: *const u8,
        key_len: isize,
        peer: *const u8,
        peer_len: isize,
        reason: *const std::ffi::c_char,
        out: *mut u8,
        out_cap: isize,
    ) -> isize;
}

/// A new enclave key: its blob (`dataRepresentation`) and its X9.63 public
/// key.
#[derive(Debug, Clone)]
pub struct EnclaveKey {
    pub blob: Vec<u8>,
    pub public: Vec<u8>,
}

fn failure(code: isize) -> PlatformError {
    match code {
        -2 => PlatformError::Cancelled,
        -3 => PlatformError::Failed("an enclave buffer was too small".into()),
        _ => PlatformError::Failed("the Secure Enclave refused".into()),
    }
}

/// Creates a key agreement key in the Secure Enclave. With `presence`,
/// each use asks for Touch ID or the account password.
pub fn create(presence: bool) -> Result<EnclaveKey, PlatformError> {
    let mut blob = vec![0u8; 4096];
    let mut public = vec![0u8; 128];
    let mut blob_len: isize = 0;
    // SAFETY: both buffers outlive the call and their capacities are passed.
    let n = unsafe {
        mado_enclave_create(
            presence,
            blob.as_mut_ptr(),
            blob.len() as isize,
            &raw mut blob_len,
            public.as_mut_ptr(),
            public.len() as isize,
        )
    };
    if n < 0 {
        return Err(failure(n));
    }
    blob.truncate(usize::try_from(blob_len).unwrap_or(0));
    public.truncate(usize::try_from(n).unwrap_or(0));
    Ok(EnclaveKey { blob, public })
}

/// The 32-byte ECDH secret of enclave key `blob` with public key `peer`.
/// Blocks while the system asks for Touch ID; call it off the main thread.
pub fn agree(blob: &[u8], peer: &[u8], reason: &str) -> Result<[u8; 32], PlatformError> {
    let reason =
        CString::new(reason).map_err(|_| PlatformError::Failed("a NUL in the reason".into()))?;
    let mut out = [0u8; 32];
    // SAFETY: the inputs and the output buffer outlive the call; lengths
    // are passed with them.
    let n = unsafe {
        mado_enclave_agree(
            blob.as_ptr(),
            blob.len() as isize,
            peer.as_ptr(),
            peer.len() as isize,
            reason.as_ptr(),
            out.as_mut_ptr(),
            out.len() as isize,
        )
    };
    match n {
        32 => Ok(out),
        n if n < 0 => Err(failure(n)),
        _ => Err(PlatformError::Failed(
            "the enclave gave a short secret".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    /// Runs on any Apple silicon Mac without a prompt: a key made without
    /// presence agrees with no Touch ID sheet.
    #[test]
    fn an_enclave_key_agrees_after_being_reloaded_from_its_blob() {
        let Ok(k) = super::create(false) else {
            eprintln!("no Secure Enclave here");
            return;
        };
        assert_eq!(k.public.len(), 65);
        let other = super::create(false).expect("a second key");
        let a = super::agree(&k.blob, &other.public, "test").expect("agree");
        let b = super::agree(&other.blob, &k.public, "test").expect("agree back");
        assert_eq!(a, b, "ECDH is symmetric");
        assert!(super::agree(&k.blob[1..], &other.public, "test").is_err());
    }
}
