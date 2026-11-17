//! `VK` wrapped by a Secure Enclave key (spec 11 §2.1): an ephemeral P-256
//! key agreement with the enclave key, HKDF-SHA256 with info
//! `mado vault key`, then AES-256-GCM. The profile folder holds
//! `vault.se-key` (the enclave key's blob), `vault.eph-pub` and
//! `vault.wrapped` (nonce then ciphertext), each mode 0600. Unwrapping asks
//! the enclave, which shows the Touch ID sheet itself.

use std::path::Path;

use p256::{PublicKey, SecretKey};
use zeroize::Zeroizing;

use crate::crypto::{hkdf, open, seal};
use crate::{VaultError, VaultKey};

const SE_KEY: &str = "vault.se-key";
const EPH_PUB: &str = "vault.eph-pub";
const WRAPPED: &str = "vault.wrapped";
const INFO: &[u8] = b"mado vault key";
const AAD: &[u8] = b"mado-vault-wrap-v1";

/// The key store that holds the wrapping key.
pub trait Enclave: Send + Sync + std::fmt::Debug {
    /// A new wrapping key: `(blob, X9.63 public key)`. Using it later asks
    /// the person.
    fn create(&self) -> Result<(Vec<u8>, Vec<u8>), VaultError>;
    /// The ECDH secret of the key in `blob` with `peer`, after asking the
    /// person with `reason`. Blocks.
    fn agree(
        &self,
        blob: &[u8],
        peer: &[u8],
        reason: &str,
    ) -> Result<Zeroizing<[u8; 32]>, VaultError>;
}

/// This Mac's Secure Enclave, with Touch ID or the account password.
#[derive(Debug, Default)]
pub struct SecureEnclave;

impl Enclave for SecureEnclave {
    fn create(&self) -> Result<(Vec<u8>, Vec<u8>), VaultError> {
        let k = mado_platform::enclave::create(true)?;
        Ok((k.blob, k.public))
    }

    fn agree(
        &self,
        blob: &[u8],
        peer: &[u8],
        reason: &str,
    ) -> Result<Zeroizing<[u8; 32]>, VaultError> {
        Ok(Zeroizing::new(mado_platform::enclave::agree(
            blob, peer, reason,
        )?))
    }
}

/// A software stand-in for tests: each instance is one "Mac", whose blobs
/// another instance cannot use. `cancel` makes the next `agree` act as a
/// cancelled sheet.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug)]
pub struct SoftEnclave {
    machine: [u8; 32],
    pub cancel: std::sync::atomic::AtomicBool,
    pub asked: std::sync::atomic::AtomicUsize,
}

#[cfg(any(test, feature = "testing"))]
impl Default for SoftEnclave {
    fn default() -> Self {
        Self {
            machine: crate::random(),
            cancel: false.into(),
            asked: 0.into(),
        }
    }
}

#[cfg(any(test, feature = "testing"))]
impl Enclave for SoftEnclave {
    fn create(&self) -> Result<(Vec<u8>, Vec<u8>), VaultError> {
        let secret = new_secret()?;
        let public = secret.public_key().to_sec1_bytes().to_vec();
        let (nonce, data) = seal(&self.machine, &secret.to_bytes(), b"soft-enclave");
        Ok(([nonce, data].concat(), public))
    }

    fn agree(
        &self,
        blob: &[u8],
        peer: &[u8],
        _reason: &str,
    ) -> Result<Zeroizing<[u8; 32]>, VaultError> {
        use std::sync::atomic::Ordering;
        self.asked.fetch_add(1, Ordering::SeqCst);
        if self.cancel.swap(false, Ordering::SeqCst) {
            return Err(VaultError::Cancelled);
        }
        if blob.len() < 12 {
            return Err(VaultError::BadKey);
        }
        let raw = open(&self.machine, &blob[..12], &blob[12..], b"soft-enclave")?;
        let secret = SecretKey::from_slice(&raw).map_err(|_| VaultError::BadKey)?;
        Ok(ecdh(&secret, &public_key(peer)?))
    }
}

fn public_key(x963: &[u8]) -> Result<PublicKey, VaultError> {
    PublicKey::from_sec1_bytes(x963).map_err(|_| VaultError::BadKey)
}

/// A random P-256 private key (32 random bytes are a valid scalar with
/// overwhelming probability; retried otherwise).
fn new_secret() -> Result<SecretKey, VaultError> {
    for _ in 0..4 {
        if let Ok(k) = SecretKey::from_slice(&crate::random::<32>()) {
            return Ok(k);
        }
    }
    Err(VaultError::BadKey)
}

/// The raw ECDH secret (the shared point's x coordinate), as CryptoKit's
/// `SharedSecret` holds it.
fn ecdh(secret: &SecretKey, peer: &PublicKey) -> Zeroizing<[u8; 32]> {
    let shared = p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), peer.as_affine());
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(shared.raw_secret_bytes());
    out
}

/// Writes a file only this user may read, replacing it in one rename.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)
}

/// Whether `dir` holds a wrapped key.
pub fn exists(dir: &Path) -> bool {
    dir.join(WRAPPED).exists()
}

/// Wraps `vk` with a new enclave key and writes the three files.
pub fn store(enclave: &dyn Enclave, dir: &Path, vk: &VaultKey) -> Result<(), VaultError> {
    let (blob, enclave_public) = enclave.create()?;
    let eph = new_secret()?;
    let eph_public = eph.public_key().to_sec1_bytes().to_vec();
    let shared = ecdh(&eph, &public_key(&enclave_public)?);
    let key = hkdf(shared.as_ref(), INFO);
    let (nonce, data) = seal(&key, vk.as_ref(), AAD);
    write_private(&dir.join(SE_KEY), &blob)?;
    write_private(&dir.join(EPH_PUB), &eph_public)?;
    write_private(&dir.join(WRAPPED), &[nonce, data].concat())?;
    Ok(())
}

/// Reads `VK`, asking the person through the enclave.
pub fn load(enclave: &dyn Enclave, dir: &Path, reason: &str) -> Result<VaultKey, VaultError> {
    if !exists(dir) {
        return Err(VaultError::NotSetUp);
    }
    let blob = std::fs::read(dir.join(SE_KEY))?;
    let eph_public = std::fs::read(dir.join(EPH_PUB))?;
    let wrapped = std::fs::read(dir.join(WRAPPED))?;
    if wrapped.len() < 12 {
        return Err(VaultError::BadKey);
    }
    let shared = enclave.agree(&blob, &eph_public, reason)?;
    let key = hkdf(shared.as_ref(), INFO);
    let raw = open(&key, &wrapped[..12], &wrapped[12..], AAD)?;
    let mut vk = Zeroizing::new([0u8; 32]);
    if raw.len() != 32 {
        return Err(VaultError::BadKey);
    }
    vk.copy_from_slice(&raw);
    Ok(vk)
}

/// Removes the wrapped key files (the vault is reset or the profile goes).
pub fn remove(dir: &Path) {
    for f in [SE_KEY, EPH_PUB, WRAPPED] {
        let _ = std::fs::remove_file(dir.join(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapped_key_opens_only_on_its_own_enclave() {
        let dir = tempfile::tempdir().expect("dir");
        let mac = SoftEnclave::default();
        let vk = Zeroizing::new(crate::random::<32>());
        store(&mac, dir.path(), &vk).expect("stored");
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(dir.path().join(WRAPPED))
                .expect("file")
                .permissions(),
        );
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(*load(&mac, dir.path(), "test").expect("unwraps"), *vk);
        let other = SoftEnclave::default();
        assert!(matches!(
            load(&other, dir.path(), "test"),
            Err(VaultError::BadKey)
        ));
        mac.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            load(&mac, dir.path(), "test"),
            Err(VaultError::Cancelled)
        ));
    }

    /// The real enclave, with a key made without presence so no sheet shows.
    #[test]
    fn the_secure_enclave_wraps_and_unwraps() {
        #[derive(Debug)]
        struct NoPrompt;
        impl Enclave for NoPrompt {
            fn create(&self) -> Result<(Vec<u8>, Vec<u8>), VaultError> {
                let k = mado_platform::enclave::create(false)?;
                Ok((k.blob, k.public))
            }
            fn agree(
                &self,
                blob: &[u8],
                peer: &[u8],
                reason: &str,
            ) -> Result<Zeroizing<[u8; 32]>, VaultError> {
                SecureEnclave.agree(blob, peer, reason)
            }
        }
        if mado_platform::enclave::create(false).is_err() {
            eprintln!("no Secure Enclave here");
            return;
        }
        let dir = tempfile::tempdir().expect("dir");
        let vk = Zeroizing::new(crate::random::<32>());
        store(&NoPrompt, dir.path(), &vk).expect("stored");
        assert_eq!(*load(&NoPrompt, dir.path(), "test").expect("unwraps"), *vk);
    }
}
