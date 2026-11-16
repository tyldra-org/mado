//! Mado Passwords storage and cryptography (spec 11 §2): the vault key and
//! its Secure Enclave wrapping, the recovery secret, encrypted items with
//! their HMAC index, never-save tags and Recently deleted.
//!
//! The vault key `VK` never touches the disk in the clear. It is wrapped by
//! a Secure Enclave key (`wrap`), and also by a key derived from the
//! recovery secret (`recovery`). The core holds the derived keys
//! (`Keys`) while the vault is unlocked and drops them on lock.

mod crypto;
pub mod item;
pub mod recovery;
mod store;
pub mod wrap;

pub use crypto::{Keys, VaultKey};
pub use item::{Item, ItemKind};
pub use store::{DeletedItem, PURGE_AFTER_MS, Stored, Vault};

/// Why a vault operation failed.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("the vault is not set up")]
    NotSetUp,
    #[error("the vault is already set up")]
    AlreadySetUp,
    #[error("authentication was cancelled")]
    Cancelled,
    /// The wrapped key or an item did not decrypt: another Mac's files, or
    /// tampering.
    #[error("the vault key could not be opened")]
    BadKey,
    #[error("the recovery secret is wrong")]
    BadRecoverySecret,
    #[error("no such item")]
    NotFound,
    #[error("{0}")]
    Platform(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

impl From<mado_platform::PlatformError> for VaultError {
    fn from(e: mado_platform::PlatformError) -> Self {
        match e {
            mado_platform::PlatformError::Cancelled => Self::Cancelled,
            e => Self::Platform(e.to_string()),
        }
    }
}

/// Random bytes from the OS.
pub(crate) fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    // The OS generator does not fail on supported systems.
    getrandom::fill(&mut b).expect("the OS random number generator");
    b
}
