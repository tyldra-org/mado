//! IDs (spec 00, conventions): UUIDv7 newtypes, serialised as lowercase
//! hyphenated strings, and the Chrome extension ID.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;
use uuid::Uuid;

macro_rules! uuid_id {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {$(
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
        #[ts(export)]
        pub struct $name(Uuid);

        impl $name {
            /// A new time-ordered ID.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.hyphenated().fmt(f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0.hyphenated())
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    )*};
}

uuid_id! {
    /// A profile, and the WebKit data store of the same UUID.
    ProfileId,
    WindowId,
    TabId,
    GroupId,
    DownloadId,
    VaultItemId,
    /// An AI agent task.
    TaskId,
    PromptId,
    /// A paired control client (spec 14).
    ClientId,
}

/// A Chrome extension ID: 32 letters `a` to `p`, the Chrome Web Store ID
/// (spec 09). Not a UUID.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, TS)]
#[ts(export)]
pub struct ExtensionId(String);

/// Why a string is not an extension ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidExtensionId;

impl fmt::Display for InvalidExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an extension ID is 32 letters from a to p")
    }
}

impl std::error::Error for InvalidExtensionId {}

impl ExtensionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ExtensionId {
    type Err = InvalidExtensionId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() == 32 && s.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
            Ok(Self(s.to_owned()))
        } else {
            Err(InvalidExtensionId)
        }
    }
}

impl<'de> Deserialize<'de> for ExtensionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ExtensionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ExtensionId({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_ids_are_lowercase_hyphenated_strings_and_time_ordered() {
        let a = TabId::new();
        let b = TabId::new();
        assert!(a < b);
        let json = serde_json::to_string(&a).expect("serialises");
        assert_eq!(json, format!("\"{a}\""));
        assert_eq!(json, json.to_lowercase());
        assert_eq!(json.matches('-').count(), 4);
        let back: TabId = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, a);
    }

    #[test]
    fn extension_ids_accept_only_32_letters_a_to_p() {
        let ok = "cjpalhdlnbpafiamejdnhcphjbkeiagm";
        assert_eq!(
            ok.parse::<ExtensionId>().map(|e| e.to_string()),
            Ok(ok.to_owned())
        );
        for bad in [
            "",
            "cjpalhdlnbpafiamejdnhcphjbkeiag",
            "cjpalhdlnbpafiamejdnhcphjbkeiagz",
            "CJPALHDLNBPAFIAMEJDNHCPHJBKEIAGM",
        ] {
            assert!(bad.parse::<ExtensionId>().is_err(), "{bad}");
            assert!(serde_json::from_value::<ExtensionId>(serde_json::json!(bad)).is_err());
        }
    }
}
