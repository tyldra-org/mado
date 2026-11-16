//! The recovery secret (spec 11 §2.1): 128 random bits shown once as 26
//! base32 characters in groups of 5. `RK = Argon2id(secret, salt, m=64 MiB,
//! t=3, p=1)` encrypts a second copy of `VK` in `vault_meta`, so a new Mac
//! or a reset Secure Enclave can still open the vault.

use zeroize::Zeroizing;

use crate::VaultError;

const ALPHABET: data_encoding::Encoding = data_encoding::BASE32_NOPAD;

/// 16 random bytes and their text form `ABCDE-FGHIJ-KLMNO-PQRST-UVWXY-Z`.
pub fn generate() -> (Zeroizing<[u8; 16]>, String) {
    let bytes = Zeroizing::new(crate::random::<16>());
    (bytes.clone(), format(&bytes))
}

/// The grouped text form of a secret.
pub fn format(bytes: &[u8; 16]) -> String {
    let text = ALPHABET.encode(bytes);
    text.as_bytes()
        .chunks(5)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join("-")
}

/// Reads a secret as typed: any case, spaces and dashes ignored.
pub fn parse(text: &str) -> Result<Zeroizing<[u8; 16]>, VaultError> {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let raw = ALPHABET
        .decode(clean.as_bytes())
        .map_err(|_| VaultError::BadRecoverySecret)?;
    let bytes: [u8; 16] = raw.try_into().map_err(|_| VaultError::BadRecoverySecret)?;
    Ok(Zeroizing::new(bytes))
}

/// `RK` from the secret and its salt.
pub(crate) fn derive(secret: &[u8; 16], salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    let params = argon2::Params::new(64 * 1024, 3, 1, Some(32))
        .map_err(|_| VaultError::BadRecoverySecret)?;
    let mut out = Zeroizing::new([0u8; 32]);
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(secret, salt, out.as_mut())
        .map_err(|_| VaultError::BadRecoverySecret)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_secret_reads_back_as_typed() {
        let (bytes, text) = super::generate();
        assert_eq!(text.len(), 26 + 5);
        assert_eq!(text.split('-').count(), 6);
        assert_eq!(*super::parse(&text).expect("parses"), *bytes);
        let sloppy = text.to_lowercase().replace('-', " ");
        assert_eq!(*super::parse(&sloppy).expect("parses"), *bytes);
        assert!(super::parse("not a secret").is_err());
    }
}
