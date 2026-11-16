//! Chrome Web Store extensions (spec 09): the store client, CRX3
//! verification, safe unpacking, manifests and overlaps.

pub mod crx;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
pub mod id;
pub mod manifest;
pub mod store;
pub mod unzip;

pub use crx::{CrxError, Expect, Verified, verify};
pub use id::{id_from_key, id_from_path, is_extension_id};
pub use manifest::{Manifest, Refusal, SearchProvider, Summary};
pub use unzip::{Unpacked, UnzipError, unzip};
