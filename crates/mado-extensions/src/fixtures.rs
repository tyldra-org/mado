//! Test packages: CRX3 files signed with test keys and ZIPs, for this
//! crate's tests and the core's (feature `fixtures`).

use std::io::Write as _;

use p256::ecdsa::signature::hazmat::PrehashSigner as _;
use prost::Message as _;
use rsa::pkcs8::{DecodePrivateKey as _, EncodePublicKey as _};
use sha2::{Digest, Sha256};

use crate::crx::{AsymmetricKeyProof, CONTEXT, CrxFileHeader, SignedData};

/// SHA-256 of a public key, as publisher keys are pinned.
pub fn key_hash(spki: &[u8]) -> [u8; 32] {
    Sha256::digest(spki).into()
}

/// A ZIP of `(name, contents)`.
pub fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in entries {
        w.start_file(*name, options).expect("start");
        w.write_all(body).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

#[derive(Debug)]
pub enum Signer {
    Rsa(Box<rsa::PrivateKey>),
    Ec(p256::ecdsa::SigningKey),
}

impl Signer {
    pub fn rsa() -> Self {
        let der = include_bytes!("../testdata/developer-rsa-test-only.der");
        Self::Rsa(Box::new(
            rsa::PrivateKey::from_pkcs8_der(der).expect("test key"),
        ))
    }
    pub fn ec(seed: u8) -> Self {
        Self::Ec(p256::ecdsa::SigningKey::from_slice(&[seed; 32]).expect("test key"))
    }
    pub fn public_key(&self) -> Vec<u8> {
        match self {
            Self::Rsa(k) => k.to_public_key().to_public_key_der().expect("der").to_vec(),
            Self::Ec(k) => k.verifying_key().to_public_key_der().expect("der").to_vec(),
        }
    }
    fn proof(&self, digest: &[u8]) -> AsymmetricKeyProof {
        let signature = match self {
            Self::Rsa(k) => {
                let k = rsa::pkcs1v15::SigningKey::<Sha256>::new((**k).clone());
                let s: rsa::pkcs1v15::Signature = k.sign_prehash(digest).expect("sign");
                Box::<[u8]>::from(s).to_vec()
            }
            Self::Ec(k) => {
                let s: p256::ecdsa::Signature = k.sign_prehash(digest).expect("sign");
                s.to_der().as_bytes().to_vec()
            }
        };
        AsymmetricKeyProof {
            public_key: Some(self.public_key()),
            signature: Some(signature),
        }
    }
}

/// A CRX3 of `zip` signed by `signers`, declaring the ID of `developer`.
pub fn crx(signers: &[&Signer], developer: &Signer, zip: &[u8], extra: &[u8]) -> Vec<u8> {
    let crx_id = Sha256::digest(developer.public_key())[..16].to_vec();
    let signed = SignedData {
        crx_id: Some(crx_id),
    }
    .encode_to_vec();
    let mut hasher = Sha256::new();
    hasher.update(CONTEXT);
    hasher.update(u32::try_from(signed.len()).expect("len").to_le_bytes());
    hasher.update(&signed);
    hasher.update(zip);
    let digest = hasher.finalize();
    let mut header = CrxFileHeader {
        signed_header_data: Some(signed),
        ..CrxFileHeader::default()
    };
    for s in signers {
        match s {
            Signer::Rsa(_) => header.sha256_with_rsa.push(s.proof(&digest)),
            Signer::Ec(_) => header.sha256_with_ecdsa.push(s.proof(&digest)),
        }
    }
    let mut header = header.encode_to_vec();
    header.extend_from_slice(extra);
    let mut out = b"Cr24".to_vec();
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(header.len()).expect("len").to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(zip);
    out
}
