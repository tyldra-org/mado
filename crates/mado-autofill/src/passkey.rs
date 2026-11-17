//! Mado Passwords passkeys (spec 11 §8): the WebAuthn authenticator's
//! side. Creation builds `clientDataJSON`, `authenticatorData` and a
//! `none` attestation; assertion signs `authenticatorData ‖
//! SHA-256(clientDataJSON)` with ECDSA P-256. Byte strings travel as
//! base64url without padding.

use data_encoding::BASE64URL_NOPAD;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::{DecodePrivateKey as _, EncodePrivateKey as _, EncodePublicKey as _};
use sha2::{Digest as _, Sha256};

/// Mado's AAGUID, the same for every passkey it makes.
pub const AAGUID: [u8; 16] = [
    0x6d, 0x61, 0x64, 0x6f, 0x2d, 0x70, 0x61, 0x73, 0x73, 0x77, 0x6f, 0x72, 0x64, 0x73, 0x2d, 0x31,
];

const UP: u8 = 0x01;
const UV: u8 = 0x04;
const BE: u8 = 0x08;
const BS: u8 = 0x10;
const AT: u8 = 0x40;

pub fn b64(b: &[u8]) -> String {
    BASE64URL_NOPAD.encode(b)
}

pub fn unb64(s: &str) -> Option<Vec<u8>> {
    BASE64URL_NOPAD
        .decode(s.trim_end_matches('=').as_bytes())
        .ok()
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PasskeyError {
    /// The page's request is one Mado does not answer; the script falls
    /// through to the original API.
    #[error("{0}")]
    NotSupported(&'static str),
    /// The page is not allowed this RP ID.
    #[error("the RP ID does not match the page")]
    Security,
}

/// Whether `origin` may use `rp_id` (11 §8.2 step 2): https (or
/// localhost), and the RP ID is its host or a registrable suffix of it.
pub fn check_rp(origin: &str, rp_id: &str) -> Result<(), PasskeyError> {
    let url = url::Url::parse(origin).map_err(|_| PasskeyError::Security)?;
    let host = url
        .host_str()
        .ok_or(PasskeyError::Security)?
        .to_ascii_lowercase();
    let local = host == "localhost" || host.ends_with(".localhost");
    if url.scheme() != "https" && !(local && url.scheme() == "http") {
        return Err(PasskeyError::Security);
    }
    let rp = rp_id.to_ascii_lowercase();
    if rp == host {
        return Ok(());
    }
    let suffix = host
        .strip_suffix(&rp)
        .is_some_and(|rest| rest.ends_with('.'));
    // A public suffix (`com`, `co.uk`) is never an RP ID.
    let public = psl::suffix_str(&rp) == Some(rp.as_str());
    if suffix && !public {
        Ok(())
    } else {
        Err(PasskeyError::Security)
    }
}

/// `clientDataJSON`, as WebAuthn serialises it.
pub fn client_data(kind: &str, challenge_b64: &str, origin: &str) -> Vec<u8> {
    format!(
        r#"{{"type":"{kind}","challenge":"{challenge_b64}","origin":{},"crossOrigin":false}}"#,
        serde_json::Value::String(origin.to_owned())
    )
    .into_bytes()
}

/// Minimal CBOR for COSE keys and attestation objects.
mod cbor {
    fn head(out: &mut Vec<u8>, major: u8, n: u64) {
        let m = major << 5;
        match n {
            0..=23 => out.push(m | n as u8),
            24..=0xff => out.extend([m | 24, n as u8]),
            0x100..=0xffff => {
                out.push(m | 25);
                out.extend((n as u16).to_be_bytes());
            }
            _ => {
                out.push(m | 26);
                out.extend((n as u32).to_be_bytes());
            }
        }
    }
    pub fn int(out: &mut Vec<u8>, v: i64) {
        if v >= 0 {
            head(out, 0, v.unsigned_abs());
        } else {
            head(out, 1, (-1 - v).unsigned_abs());
        }
    }
    pub fn bytes(out: &mut Vec<u8>, b: &[u8]) {
        head(out, 2, b.len() as u64);
        out.extend(b);
    }
    pub fn text(out: &mut Vec<u8>, s: &str) {
        head(out, 3, s.len() as u64);
        out.extend(s.as_bytes());
    }
    pub fn map(out: &mut Vec<u8>, entries: usize) {
        head(out, 5, entries as u64);
    }
}

/// The COSE_Key of a P-256 public key (EC2, ES256).
pub fn cose_key(public: &p256::PublicKey) -> Vec<u8> {
    let point = public.to_sec1_bytes();
    let (x, y) = (&point[1..33], &point[33..65]);
    let mut out = Vec::new();
    cbor::map(&mut out, 5);
    cbor::int(&mut out, 1);
    cbor::int(&mut out, 2);
    cbor::int(&mut out, 3);
    cbor::int(&mut out, -7);
    cbor::int(&mut out, -1);
    cbor::int(&mut out, 1);
    cbor::int(&mut out, -2);
    cbor::bytes(&mut out, x);
    cbor::int(&mut out, -3);
    cbor::bytes(&mut out, y);
    out
}

fn auth_data(rp_id: &str, flags: u8, attested: Option<(&[u8], &p256::PublicKey)>) -> Vec<u8> {
    let mut out = Sha256::digest(rp_id.as_bytes()).to_vec();
    out.push(flags);
    out.extend(0u32.to_be_bytes());
    if let Some((id, key)) = attested {
        out.extend(AAGUID);
        out.extend(u16::try_from(id.len()).unwrap_or(u16::MAX).to_be_bytes());
        out.extend(id);
        out.extend(cose_key(key));
    }
    out
}

/// A new passkey and the page's registration response.
#[derive(Clone, PartialEq, Eq)]
pub struct Created {
    pub credential_id: Vec<u8>,
    /// PKCS#8 DER of the private key, for the vault.
    pub private_key: Vec<u8>,
    pub client_data_json: Vec<u8>,
    pub attestation_object: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    /// SPKI DER, for `getPublicKey()`.
    pub public_key: Vec<u8>,
}

impl std::fmt::Debug for Created {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Created")
            .field("credential_id", &b64(&self.credential_id))
            .finish_non_exhaustive()
    }
}

/// 11 §8.2 step 4: a new key and its attestation.
pub fn create(rp_id: &str, challenge_b64: &str, origin: &str) -> Result<Created, PasskeyError> {
    let mut seed = [0u8; 32];
    let secret = loop {
        getrandom::fill(&mut seed).map_err(|_| PasskeyError::NotSupported("no randomness"))?;
        if let Ok(k) = p256::SecretKey::from_slice(&seed) {
            break k;
        }
    };
    let mut credential_id = vec![0u8; 16];
    getrandom::fill(&mut credential_id).map_err(|_| PasskeyError::NotSupported("no randomness"))?;
    let public = secret.public_key();
    let authenticator_data = auth_data(
        rp_id,
        UP | UV | BE | BS | AT,
        Some((&credential_id, &public)),
    );
    let mut attestation_object = Vec::new();
    cbor::map(&mut attestation_object, 3);
    cbor::text(&mut attestation_object, "fmt");
    cbor::text(&mut attestation_object, "none");
    cbor::text(&mut attestation_object, "attStmt");
    cbor::map(&mut attestation_object, 0);
    cbor::text(&mut attestation_object, "authData");
    cbor::bytes(&mut attestation_object, &authenticator_data);
    Ok(Created {
        credential_id,
        private_key: secret
            .to_pkcs8_der()
            .map_err(|_| PasskeyError::NotSupported("key encoding"))?
            .as_bytes()
            .to_vec(),
        client_data_json: client_data("webauthn.create", challenge_b64, origin),
        attestation_object,
        authenticator_data,
        public_key: public
            .to_public_key_der()
            .map_err(|_| PasskeyError::NotSupported("key encoding"))?
            .as_bytes()
            .to_vec(),
    })
}

/// An assertion for the page.
#[derive(Clone, PartialEq, Eq)]
pub struct Assertion {
    pub client_data_json: Vec<u8>,
    pub authenticator_data: Vec<u8>,
    /// DER ECDSA signature.
    pub signature: Vec<u8>,
}

impl std::fmt::Debug for Assertion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Assertion(..)")
    }
}

/// 11 §8.3 step 3: signs with the stored PKCS#8 key.
pub fn assert(
    private_key_pkcs8: &[u8],
    rp_id: &str,
    challenge_b64: &str,
    origin: &str,
) -> Result<Assertion, PasskeyError> {
    let key = SigningKey::from_pkcs8_der(private_key_pkcs8)
        .map_err(|_| PasskeyError::NotSupported("a damaged key"))?;
    let authenticator_data = auth_data(rp_id, UP | UV | BE | BS, None);
    let client_data_json = client_data("webauthn.get", challenge_b64, origin);
    let mut message = authenticator_data.clone();
    message.extend(Sha256::digest(&client_data_json));
    let signature: Signature = key.sign(&message);
    Ok(Assertion {
        client_data_json,
        authenticator_data,
        signature: signature.to_der().as_bytes().to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::VerifyingKey;
    use p256::ecdsa::signature::Verifier as _;
    use p256::pkcs8::DecodePublicKey as _;

    #[test]
    fn rp_ids_follow_the_origin() {
        assert!(check_rp("https://login.example.co.uk", "example.co.uk").is_ok());
        assert!(check_rp("https://login.example.co.uk", "login.example.co.uk").is_ok());
        assert!(
            check_rp("https://login.example.co.uk", "co.uk").is_err(),
            "a public suffix"
        );
        assert!(check_rp("https://example.com", "other.com").is_err());
        assert!(check_rp("https://badexample.com", "example.com").is_err());
        assert!(
            check_rp("http://example.com", "example.com").is_err(),
            "http"
        );
        assert!(check_rp("http://localhost:8080", "localhost").is_ok());
    }

    #[test]
    fn a_created_passkey_signs_assertions_the_site_can_verify() {
        let made = create("example.com", "Y2hhbGxlbmdl", "https://example.com").expect("created");
        // authenticatorData: rpIdHash, flags, counter, AAGUID, id, COSE key.
        let ad = &made.authenticator_data;
        assert_eq!(&ad[..32], Sha256::digest(b"example.com").as_slice());
        assert_eq!(ad[32], 0x5d);
        assert_eq!(&ad[33..37], &[0, 0, 0, 0]);
        assert_eq!(&ad[37..53], &AAGUID);
        assert_eq!(&ad[53..55], &[0, 16]);
        assert_eq!(&ad[55..71], made.credential_id.as_slice());
        let spki = VerifyingKey::from_public_key_der(&made.public_key).expect("spki");
        let cose = cose_key(&p256::PublicKey::from_public_key_der(&made.public_key).expect("pk"));
        assert_eq!(&ad[71..], cose.as_slice());
        assert_eq!(cose[0], 0xa5, "a map of five");
        // The attestation object is {"fmt":"none","attStmt":{},"authData":…}.
        assert!(
            made.attestation_object
                .starts_with(&[0xa3, 0x63, b'f', b'm', b't', 0x64, b'n', b'o', b'n', b'e'])
        );
        let cd: serde_json::Value = serde_json::from_slice(&made.client_data_json).expect("json");
        assert_eq!(
            (cd["type"].as_str(), cd["origin"].as_str()),
            (Some("webauthn.create"), Some("https://example.com"))
        );

        let got = assert(
            &made.private_key,
            "example.com",
            "bm9uY2U",
            "https://example.com",
        )
        .expect("asserted");
        assert_eq!(got.authenticator_data[32], 0x1d);
        let mut signed = got.authenticator_data.clone();
        signed.extend(Sha256::digest(&got.client_data_json));
        let sig = Signature::from_der(&got.signature).expect("der");
        spki.verify(&signed, &sig)
            .expect("the site verifies the signature");
        assert!(spki.verify(b"something else", &sig).is_err());
    }
}
