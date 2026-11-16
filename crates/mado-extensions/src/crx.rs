//! CRX3 verification (spec 09 §3), as Chromium's `crx_verifier.cc` does
//! it: every proof must verify, one must be the developer key the ID comes
//! from, and store installs need the Web Store's publisher proof.

use p256::ecdsa::signature::hazmat::PrehashVerifier as _;
use prost::Message;
use rsa::pkcs8::DecodePublicKey as _;
use sha2::{Digest, Sha256};

use crate::id::{id_from_hash_prefix, id_from_key};

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct CrxFileHeader {
    #[prost(message, repeated, tag = "2")]
    pub(crate) sha256_with_rsa: Vec<AsymmetricKeyProof>,
    #[prost(message, repeated, tag = "3")]
    pub(crate) sha256_with_ecdsa: Vec<AsymmetricKeyProof>,
    #[prost(bytes = "vec", optional, tag = "10000")]
    pub(crate) signed_header_data: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct AsymmetricKeyProof {
    #[prost(bytes = "vec", optional, tag = "1")]
    pub(crate) public_key: Option<Vec<u8>>,
    #[prost(bytes = "vec", optional, tag = "2")]
    pub(crate) signature: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub(crate) struct SignedData {
    #[prost(bytes = "vec", optional, tag = "1")]
    pub(crate) crx_id: Option<Vec<u8>>,
}

const MAGIC: &[u8] = b"Cr24";
const MAX_HEADER: usize = 1024 * 1024;
pub(crate) const CONTEXT: &[u8] = b"CRX3 SignedData\0";

/// SHA-256 of the Chrome Web Store's publisher key (DER SPKI,
/// "ecdsa_2017_public"), Chromium's `kPublisherKeyHash` in
/// `components/crx_file/crx_verifier.cc`.
pub const PUBLISHER_KEY_HASH: [u8; 32] = [
    0x61, 0xf7, 0xf2, 0xa6, 0xbf, 0xcf, 0x74, 0xcd, 0x0b, 0xc1, 0xfe, 0x24, 0x97, 0xcc, 0x9b, 0x04,
    0x25, 0x4c, 0x65, 0x8f, 0x79, 0xf2, 0x14, 0x53, 0x92, 0x86, 0x7e, 0xa8, 0x36, 0x63, 0x67, 0xcf,
];

/// What a package must be.
#[derive(Debug, Clone, Copy)]
pub struct Expect<'a> {
    /// The ID asked for; `None` for a `.crx` file in Developer mode.
    pub id: Option<&'a str>,
    /// Require the Web Store's publisher proof (store installs).
    pub from_store: bool,
    /// Publisher key hashes accepted besides the store's: tests only.
    pub extra_publishers: &'a [[u8; 32]],
}

/// A package that verified.
#[derive(Debug)]
pub struct Verified<'a> {
    pub id: String,
    /// The developer key, DER SubjectPublicKeyInfo.
    pub public_key: Vec<u8>,
    /// The ZIP archive.
    pub zip: &'a [u8],
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CrxError {
    #[error("not a CRX file")]
    NotCrx,
    #[error("CRX version {0} is not supported")]
    Version(u32),
    #[error("the header is malformed")]
    Header,
    #[error("a key is malformed")]
    Key,
    #[error("a signature does not verify")]
    Signature,
    #[error("no proof from the developer key")]
    NoDeveloperProof,
    #[error("no proof from the Chrome Web Store")]
    NoStoreProof,
    #[error("the package is {got}, not {expected}")]
    WrongId { expected: String, got: String },
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, CrxError> {
    let b = bytes.get(at..at + 4).ok_or(CrxError::Header)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Checks a CRX3 package and returns its archive.
pub fn verify<'a>(bytes: &'a [u8], expect: Expect<'_>) -> Result<Verified<'a>, CrxError> {
    if bytes.get(..4) != Some(MAGIC) {
        return Err(CrxError::NotCrx);
    }
    let version = u32_at(bytes, 4)?;
    if version != 3 {
        return Err(CrxError::Version(version));
    }
    let len = usize::try_from(u32_at(bytes, 8)?).map_err(|_| CrxError::Header)?;
    if len > MAX_HEADER {
        return Err(CrxError::Header);
    }
    let header_bytes = bytes.get(12..12 + len).ok_or(CrxError::Header)?;
    let zip = &bytes[12 + len..];
    // A ZIP end-of-directory record in the header could make unzipping read
    // something other than what was signed (Chromium refuses it too).
    if [b"PK\x05\x06", b"PK\x06\x06", b"PK\x06\x07"]
        .iter()
        .any(|eocd| contains(header_bytes, *eocd))
    {
        return Err(CrxError::Header);
    }
    let header = CrxFileHeader::decode(header_bytes).map_err(|_| CrxError::Header)?;
    let signed = header.signed_header_data.unwrap_or_default();
    let declared = SignedData::decode(signed.as_slice())
        .map_err(|_| CrxError::Header)?
        .crx_id
        .filter(|id| id.len() == 16)
        .ok_or(CrxError::Header)?;
    let declared = id_from_hash_prefix(&declared);

    let mut hasher = Sha256::new();
    hasher.update(CONTEXT);
    hasher.update(
        u32::try_from(signed.len())
            .map_err(|_| CrxError::Header)?
            .to_le_bytes(),
    );
    hasher.update(&signed);
    hasher.update(zip);
    let digest = hasher.finalize();

    let mut developer = None;
    let mut store = false;
    for (proof, rsa) in header
        .sha256_with_rsa
        .iter()
        .map(|p| (p, true))
        .chain(header.sha256_with_ecdsa.iter().map(|p| (p, false)))
    {
        let key = proof.public_key.as_deref().ok_or(CrxError::Key)?;
        let signature = proof.signature.as_deref().ok_or(CrxError::Signature)?;
        if rsa {
            let key = rsa::PublicKey::from_public_key_der(key).map_err(|_| CrxError::Key)?;
            let key = rsa::pkcs1v15::VerifyingKey::<Sha256>::new(key);
            let signature =
                rsa::pkcs1v15::Signature::try_from(signature).map_err(|_| CrxError::Signature)?;
            key.verify_prehash(&digest, &signature)
                .map_err(|_| CrxError::Signature)?;
        } else {
            let key =
                p256::ecdsa::VerifyingKey::from_public_key_der(key).map_err(|_| CrxError::Key)?;
            let signature =
                p256::ecdsa::Signature::from_der(signature).map_err(|_| CrxError::Signature)?;
            key.verify_prehash(&digest, &signature)
                .map_err(|_| CrxError::Signature)?;
        }
        if id_from_key(key) == declared {
            developer = Some(key.to_vec());
        }
        let hash: [u8; 32] = Sha256::digest(key).into();
        store |= hash == PUBLISHER_KEY_HASH || expect.extra_publishers.contains(&hash);
    }
    let public_key = developer.ok_or(CrxError::NoDeveloperProof)?;
    if expect.from_store && !store {
        return Err(CrxError::NoStoreProof);
    }
    if let Some(expected) = expect.id
        && expected != declared
    {
        return Err(CrxError::WrongId {
            expected: expected.to_owned(),
            got: declared,
        });
    }
    Ok(Verified {
        id: declared,
        public_key,
        zip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{Signer, crx, zip_of};

    const ANY: Expect<'static> = Expect {
        id: None,
        from_store: false,
        extra_publishers: &[],
    };

    /// 09 §12 check 1.
    #[test]
    fn a_changed_byte_or_a_missing_store_proof_is_refused() {
        let dev = Signer::rsa();
        let id = id_from_key(&dev.public_key());
        let zip = zip_of(&[(
            "manifest.json",
            br#"{"name":"x","version":"1","manifest_version":3}"#,
        )]);
        let good = crx(&[&dev], &dev, &zip, &[]);
        let ok = verify(
            &good,
            Expect {
                id: Some(&id),
                from_store: false,
                extra_publishers: &[],
            },
        )
        .expect("verifies");
        assert_eq!((ok.id.as_str(), ok.zip), (id.as_str(), zip.as_slice()));
        assert_eq!(ok.public_key, dev.public_key());

        let mut changed = good.clone();
        let last = changed.len() - 10;
        changed[last] ^= 1;
        assert_eq!(verify(&changed, ANY).map(drop), Err(CrxError::Signature));

        assert_eq!(
            verify(
                &good,
                Expect {
                    id: Some(&id),
                    from_store: true,
                    extra_publishers: &[],
                }
            )
            .map(drop),
            Err(CrxError::NoStoreProof)
        );
        // A second proof from another key is not the store's either.
        let other = Signer::ec(7);
        let two = crx(&[&dev, &other], &dev, &zip, &[]);
        assert!(verify(&two, ANY).is_ok());
        assert_eq!(
            verify(
                &two,
                Expect {
                    id: None,
                    from_store: true,
                    extra_publishers: &[],
                }
            )
            .map(drop),
            Err(CrxError::NoStoreProof)
        );
    }

    #[test]
    fn the_id_and_the_developer_proof_must_match() {
        let dev = Signer::ec(1);
        let zip = zip_of(&[("a", b"1")]);
        let good = crx(&[&dev], &dev, &zip, &[]);
        assert!(matches!(
            verify(
                &good,
                Expect {
                    id: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                    from_store: false,
                    extra_publishers: &[],
                }
            ),
            Err(CrxError::WrongId { .. })
        ));
        // Signed by one key while declaring another's ID.
        let claimed = Signer::ec(2);
        let stolen = crx(&[&dev], &claimed, &zip, &[]);
        assert_eq!(
            verify(&stolen, ANY).map(drop),
            Err(CrxError::NoDeveloperProof)
        );
        // No proofs at all.
        let none = crx(&[], &dev, &zip, &[]);
        assert_eq!(
            verify(&none, ANY).map(drop),
            Err(CrxError::NoDeveloperProof)
        );
    }

    #[test]
    fn malformed_headers_are_refused() {
        let dev = Signer::ec(1);
        let zip = zip_of(&[("a", b"1")]);
        assert_eq!(verify(b"PK\x03\x04", ANY).map(drop), Err(CrxError::NotCrx));
        let mut v2 = crx(&[&dev], &dev, &zip, &[]);
        v2[4] = 2;
        assert_eq!(verify(&v2, ANY).map(drop), Err(CrxError::Version(2)));
        // An unknown field carrying a ZIP end-of-directory record.
        let eocd = crx(&[&dev], &dev, &zip, &[0x2a, 4, b'P', b'K', 5, 6]);
        assert_eq!(verify(&eocd, ANY).map(drop), Err(CrxError::Header));
        let mut huge = crx(&[&dev], &dev, &zip, &[]);
        huge[8..12].copy_from_slice(&(2 * 1024 * 1024u32).to_le_bytes());
        assert_eq!(verify(&huge, ANY).map(drop), Err(CrxError::Header));
        let mut short = crx(&[&dev], &dev, &zip, &[]);
        short.truncate(14);
        assert_eq!(verify(&short, ANY).map(drop), Err(CrxError::Header));
    }

    proptest::proptest! {
        #[test]
        fn no_input_panics(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512)) {
            let _ = verify(&bytes, ANY);
            let mut crx = b"Cr24\x03\0\0\0".to_vec();
            crx.extend_from_slice(&bytes);
            let _ = verify(&crx, ANY);
        }
    }
}
