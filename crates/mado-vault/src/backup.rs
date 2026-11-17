//! `.madovault` files (spec 11 §10): every item, encrypted with a key
//! from a passphrase (Argon2id, then AES-256-GCM).

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::VaultError;
use crate::crypto::{open as decrypt, seal as encrypt};
use crate::item::Item;

const AAD: &[u8] = b"madovault-v1";
const B64: data_encoding::Encoding = data_encoding::BASE64;

#[derive(Debug, Serialize, Deserialize)]
struct Kdf {
    alg: String,
    m: u32,
    t: u32,
    p: u32,
    salt: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct File {
    format: String,
    version: u32,
    kdf: Kdf,
    nonce: String,
    ciphertext: String,
}

fn key(passphrase: &str, kdf: &Kdf) -> Result<Zeroizing<[u8; 32]>, VaultError> {
    if kdf.alg != "argon2id" || kdf.m > 1024 * 1024 || kdf.t > 10 || kdf.p > 8 {
        return Err(VaultError::BadKey);
    }
    let salt = B64
        .decode(kdf.salt.as_bytes())
        .map_err(|_| VaultError::BadKey)?;
    let params =
        argon2::Params::new(kdf.m, kdf.t, kdf.p, Some(32)).map_err(|_| VaultError::BadKey)?;
    let mut out = Zeroizing::new([0u8; 32]);
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), &salt, out.as_mut())
        .map_err(|_| VaultError::BadKey)?;
    Ok(out)
}

/// A `.madovault` of `items`.
pub fn seal(items: &[Item], passphrase: &str) -> Result<Vec<u8>, VaultError> {
    let salt: [u8; 16] = crate::random();
    let kdf = Kdf {
        alg: "argon2id".into(),
        m: 64 * 1024,
        t: 3,
        p: 1,
        salt: B64.encode(&salt),
    };
    let key = key(passphrase, &kdf)?;
    let plain = Zeroizing::new(serde_json::to_vec(items)?);
    let (nonce, data) = encrypt(&key, &plain, AAD);
    Ok(serde_json::to_vec_pretty(&File {
        format: "madovault".into(),
        version: 1,
        kdf,
        nonce: B64.encode(&nonce),
        ciphertext: B64.encode(&data),
    })?)
}

/// The items of a `.madovault`; a wrong passphrase is `BadRecoverySecret`.
pub fn open(bytes: &[u8], passphrase: &str) -> Result<Vec<Item>, VaultError> {
    let file: File = serde_json::from_slice(bytes)?;
    if file.format != "madovault" || file.version != 1 {
        return Err(VaultError::BadKey);
    }
    let key = key(passphrase, &file.kdf)?;
    let nonce = B64
        .decode(file.nonce.as_bytes())
        .map_err(|_| VaultError::BadKey)?;
    let data = B64
        .decode(file.ciphertext.as_bytes())
        .map_err(|_| VaultError::BadKey)?;
    let plain = decrypt(&key, &nonce, &data, AAD).map_err(|_| VaultError::BadRecoverySecret)?;
    Ok(serde_json::from_slice(&plain)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Login, Note};

    #[test]
    fn a_vault_file_opens_only_with_its_passphrase() {
        let items = vec![
            Item::Login(Login {
                title: "x".into(),
                urls: vec!["https://example.com/".into()],
                username: "me".into(),
                password: "pw".into(),
                ..Login::default()
            }),
            Item::Note(Note {
                title: "n".into(),
                body: "b".into(),
            }),
        ];
        let file = seal(&items, "correct horse").expect("sealed");
        let text = String::from_utf8(file.clone()).expect("json");
        assert!(text.contains("\"format\": \"madovault\"") && !text.contains("example.com"));
        assert_eq!(open(&file, "correct horse").expect("opened"), items);
        assert!(matches!(
            open(&file, "wrong"),
            Err(VaultError::BadRecoverySecret)
        ));
    }
}
