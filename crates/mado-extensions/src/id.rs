//! Chrome extension IDs: the first 16 bytes of a SHA-256, in hex with
//! `0-f` written as `a-p` (spec 09 §3 step 4).

use sha2::{Digest, Sha256};

/// The ID of the extension whose public key (DER SubjectPublicKeyInfo)
/// is `spki`.
pub fn id_from_key(spki: &[u8]) -> String {
    id_from_hash_prefix(&Sha256::digest(spki)[..16])
}

/// An unpacked extension's ID without a `key`: from its folder's absolute
/// path, as Chrome does (spec 09 §2.3).
pub fn id_from_path(path: &std::path::Path) -> String {
    id_from_hash_prefix(&Sha256::digest(path.as_os_str().as_encoded_bytes())[..16])
}

/// 16 bytes as an extension ID.
pub fn id_from_hash_prefix(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|b| [b >> 4, b & 0xf])
        .map(|n| char::from(b'a' + n))
        .collect()
}

/// 32 letters from `a` to `p`.
pub fn is_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_map_hex_to_a_through_p() {
        assert_eq!(
            id_from_hash_prefix(&[
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0, 0, 0, 0, 0, 0, 0, 0xff
            ]),
            "abcdefghijklmnopaaaaaaaaaaaaaapp"
        );
        assert!(is_extension_id("ddkjiahejlhfcafbddmgiahcphecmpfh"));
        assert!(!is_extension_id("ddkjiahejlhfcafbddmgiahcphecmpfz"));
        assert!(!is_extension_id("short"));
    }
}
