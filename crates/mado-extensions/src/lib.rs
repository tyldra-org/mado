//! Chrome Web Store extensions (spec 09): the store client, CRX3
//! verification, safe unpacking, manifests and overlaps.

pub mod crx;
pub mod id;
pub mod store;
pub mod unzip;

pub use crx::{CrxError, Expect, Verified, verify};
pub use id::{id_from_key, is_extension_id};
pub use unzip::{Unpacked, UnzipError, unzip};
