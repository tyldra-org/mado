//! `VK`, its derived keys and item sealing (spec 11 §2.1, §2.2).

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::VaultError;

/// The vault key, 32 random bytes, wiped when dropped.
pub type VaultKey = Zeroizing<[u8; 32]>;

/// `EK` (item encryption) and `IK` (index HMAC), derived from `VK` with
/// HKDF-SHA256; wiped when dropped.
pub struct Keys {
    ek: Zeroizing<[u8; 32]>,
    ik: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys(..)")
    }
}

pub(crate) fn hkdf(ikm: &[u8], info: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut out = Zeroizing::new([0u8; 32]);
    // 32 bytes is far under HKDF-SHA256's limit.
    hkdf::Hkdf::<Sha256>::new(None, ikm)
        .expand(info, out.as_mut())
        .expect("32 bytes of HKDF output");
    out
}

/// AES-256-GCM with a random 96-bit nonce; returns `(nonce, ciphertext)`.
pub(crate) fn seal(key: &[u8; 32], plain: &[u8], aad: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let cipher = Aes256Gcm::new(key.into());
    let nonce: [u8; 12] = crate::random();
    // Encryption fails only for inputs over 64 GiB.
    let data = cipher
        .encrypt(&Nonce::from(nonce), Payload { msg: plain, aad })
        .expect("AES-GCM encrypts");
    (nonce.to_vec(), data)
}

pub(crate) fn open(
    key: &[u8; 32],
    nonce: &[u8],
    data: &[u8],
    aad: &[u8],
) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let nonce: [u8; 12] = nonce.try_into().map_err(|_| VaultError::BadKey)?;
    Aes256Gcm::new(key.into())
        .decrypt(&Nonce::from(nonce), Payload { msg: data, aad })
        .map(Zeroizing::new)
        .map_err(|_| VaultError::BadKey)
}

impl Keys {
    pub fn derive(vk: &[u8; 32]) -> Self {
        Self {
            ek: hkdf(vk, b"mado-vault-enc-v1"),
            ik: hkdf(vk, b"mado-vault-idx-v1"),
        }
    }

    /// Encrypts an item's JSON, bound to its ID and kind.
    pub(crate) fn seal_item(&self, id: &str, kind: &str, json: &[u8]) -> (Vec<u8>, Vec<u8>) {
        seal(&self.ek, json, &aad(id, kind))
    }

    pub(crate) fn open_item(
        &self,
        id: &str,
        kind: &str,
        nonce: &[u8],
        data: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        open(&self.ek, nonce, data, &aad(id, kind))
    }

    /// `HMAC-SHA256(IK, text)`: an index tag that names a site without
    /// storing it.
    pub fn tag(&self, text: &str) -> Vec<u8> {
        let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(self.ik.as_ref())
            .expect("HMAC takes any key length");
        mac.update(text.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }
}

fn aad(id: &str, kind: &str) -> Vec<u8> {
    [id.as_bytes(), kind.as_bytes()].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_open_only_with_their_id_and_kind() {
        let k = Keys::derive(&[7; 32]);
        let (n, d) = k.seal_item("a", "login", b"{}");
        assert_eq!(&**k.open_item("a", "login", &n, &d).expect("opens"), b"{}");
        assert!(
            k.open_item("b", "login", &n, &d).is_err(),
            "moved to another ID"
        );
        assert!(k.open_item("a", "card", &n, &d).is_err(), "relabelled");
        assert!(
            Keys::derive(&[8; 32])
                .open_item("a", "login", &n, &d)
                .is_err()
        );
        assert_eq!(k.tag("site:example.com"), k.tag("site:example.com"));
        assert_ne!(
            k.tag("site:example.com"),
            Keys::derive(&[8; 32]).tag("site:example.com")
        );
    }
}
