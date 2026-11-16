//! minisign signatures over the lists (spec 15 §7). The trusted comment
//! names the file and the time of the commit it was signed from, so one
//! list cannot pass as another and an older copy cannot replace a newer.

use minisign_verify::{PublicKey, Signature};

use crate::{List, ListError};

/// The public half of `LISTS_SIGNING_KEY`. The secret half and its
/// password are repository secrets, with a copy in the maintainer's login
/// Keychain (services `app.mado.lists-signing` and
/// `app.mado.lists-signing-password`). A new key pair needs an app release
/// before lists signed with it are accepted.
pub const PUBLIC_KEY: &str = "RWSKb9G8c1WtmJCorg9JVO8eSqIVMH+QNhqKz+/emp7LLsLuld7rGhZ4";

/// The trusted comment `lists.yml` signs with.
pub fn trusted_comment(list: List, serial: u64) -> String {
    format!("timestamp:{serial}\tfile:{}", list.file())
}

/// `timestamp` and `file` from a trusted comment.
fn fields(comment: &str) -> (Option<u64>, Option<&str>) {
    let mut serial = None;
    let mut file = None;
    for field in comment.split('\t') {
        match field.split_once(':') {
            Some(("timestamp", v)) => serial = v.parse().ok(),
            Some(("file", v)) => file = Some(v),
            _ => {}
        }
    }
    (serial, file)
}

/// The serial a signature claims, without checking it: whether a list is
/// worth downloading.
pub fn signed_serial(minisig: &str) -> Option<u64> {
    let signature = Signature::decode(minisig).ok()?;
    fields(signature.trusted_comment()).0
}

pub(crate) fn public_key(key: &str) -> Result<PublicKey, ListError> {
    PublicKey::from_base64(key).map_err(|_| ListError::Key)
}

/// Checks `minisig` over `bytes` as `list`; returns its serial.
pub(crate) fn verify(
    key: &PublicKey,
    list: List,
    bytes: &[u8],
    minisig: &str,
) -> Result<u64, ListError> {
    let file = list.file();
    let signature =
        Signature::decode(minisig).map_err(|_| ListError::MalformedSignature { file })?;
    key.verify(bytes, &signature, false)
        .map_err(|_| ListError::BadSignature { file })?;
    match fields(signature.trusted_comment()) {
        (Some(serial), Some(signed)) if signed == file => Ok(serial),
        _ => Err(ListError::WrongFile {
            file,
            signed: signature.trusted_comment().to_owned(),
        }),
    }
}
