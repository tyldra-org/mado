//! The signed lists Mado fetches without an app release (spec 09 §7,
//! 15 §7): what each holds, the signature check, the last good copies on
//! disk and the copies bundled from `lists/` at build time.
//!
//! Readers call the typed accessors ([`compat`], [`sensitive_hosts`],
//! [`top_domains`], [`tracking_params`], [`hsts_preload`], [`filter_lists`]),
//! which return
//! the bundled copy until a [`Store`] installs a newer signed one.
//! [`Derived`] keeps what a reader builds from a list until it changes.

mod current;
mod data;
mod signature;
mod store;

pub use current::{
    Derived, compat, filter_lists, hsts_preload, sensitive_hosts, serial, top_domains,
    tracking_params,
};
pub use data::{
    Blocked, Category, Compat, CompatEntry, FilterList, FilterLists, HstsPreload, ListKind,
    MinSafeEngine, SensitiveHosts, Status, TopDomainList, TrackingParams,
};
pub use signature::{PUBLIC_KEY, file_comment, signed_serial, trusted_comment, verify_file};
pub use store::{Installed, Store};

/// Where the published lists live (spec 15 §7, `lists.yml`).
pub const URL: &str = "https://tyldra-org.github.io/mado/lists/";

/// The lists Mado fetches. Payment frames arrive with passwords (M4), the
/// action lexicon and the model catalogue with AI (M5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum List {
    /// Extension statuses and categories, blocked extensions and the
    /// oldest safe engine (09 §7).
    Compat,
    /// Hosts whose pages are never indexed or sent to a model (12 §2.2).
    SensitiveHosts,
    /// Domains people know, for lookalike checks (08 §2).
    TopDomains,
    /// Parameters Copy clean link removes (08 §8).
    TrackingParams,
    /// Chromium's HSTS preload list, gzipped (08 §3).
    HstsPreload,
    /// The blocker's filter lists and resources (10 §2).
    FilterLists,
}

impl List {
    pub const ALL: [Self; 6] = [
        Self::Compat,
        Self::SensitiveHosts,
        Self::TopDomains,
        Self::TrackingParams,
        Self::HstsPreload,
        Self::FilterLists,
    ];

    /// The file name, in `lists/` and where it is published.
    pub fn file(self) -> &'static str {
        match self {
            Self::Compat => "compat.json",
            Self::SensitiveHosts => "sensitive-hosts.json",
            Self::TopDomains => "top-domains.txt",
            Self::TrackingParams => "tracking-params.json",
            Self::HstsPreload => "hsts-preload.txt.gz",
            Self::FilterLists => "filter-lists.json",
        }
    }

    pub fn from_file(file: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.file() == file)
    }

    /// The copy built into the app.
    pub fn bundled(self) -> &'static [u8] {
        match self {
            Self::Compat => include_bytes!("../../../lists/compat.json"),
            Self::SensitiveHosts => include_bytes!("../../../lists/sensitive-hosts.json"),
            Self::TopDomains => include_bytes!("../../../lists/top-domains.txt"),
            Self::TrackingParams => include_bytes!("../../../lists/tracking-params.json"),
            Self::HstsPreload => include_bytes!("../../../lists/hsts-preload.txt.gz"),
            Self::FilterLists => include_bytes!("../../../lists/filter-lists.json"),
        }
    }

    /// Whether `bytes` is a well-formed list of this kind: what CI checks
    /// before signing and the app before installing.
    pub fn check(self, bytes: &[u8]) -> Result<(), ListError> {
        current::parse_only(self, bytes).map_err(|reason| ListError::Invalid {
            file: self.file(),
            reason,
        })
    }
}

/// The serial of the bundled copies: the Unix time of the commit the
/// release was built from (`MADO_LISTS_SERIAL`, set by
/// `scripts/release/macos.sh`), 0 in other builds. Published lists carry
/// the time of the commit they were signed from, so a cached copy older
/// than the app's own never replaces it.
pub const BUNDLED_SERIAL: u64 = parse_serial(option_env!("MADO_LISTS_SERIAL"));

const fn parse_serial(s: Option<&str>) -> u64 {
    let Some(s) = s else { return 0 };
    let bytes = s.as_bytes();
    let mut n: u64 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let d = bytes[i];
        if !d.is_ascii_digit() {
            return 0;
        }
        n = n.saturating_mul(10).saturating_add((d - b'0') as u64);
        i += 1;
    }
    n
}

#[derive(Debug, thiserror::Error)]
pub enum ListError {
    #[error("the lists public key is malformed")]
    Key,
    #[error("{file}: the signature is malformed")]
    MalformedSignature { file: &'static str },
    #[error("{file}: the signature does not verify")]
    BadSignature { file: &'static str },
    #[error("{file}: signed as {signed:?}")]
    WrongFile { file: &'static str, signed: String },
    #[error("{file}: version {got} is older than {have}")]
    Older {
        file: &'static str,
        got: u64,
        have: u64,
    },
    #[error("{file}: {reason}")]
    Invalid { file: &'static str, reason: String },
    #[error("{file}: {source}")]
    Io {
        file: &'static str,
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_list_is_well_formed() {
        for list in List::ALL {
            list.check(list.bundled()).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(List::from_file(list.file()), Some(list));
        }
    }

    #[test]
    fn serials_parse_digits_only() {
        assert_eq!(parse_serial(None), 0);
        assert_eq!(parse_serial(Some("1763200000")), 1_763_200_000);
        assert_eq!(parse_serial(Some("12a")), 0);
    }
}
