//! The copy of each list readers see: the bundled one until a store
//! installs a newer signed one.

use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use crate::data::{
    Compat, FilterLists, HstsPreload, PaymentFrames, SensitiveHosts, TopDomainList, TrackingParams,
};
use crate::{BUNDLED_SERIAL, List};

type Current<T> = Option<(u64, Arc<T>)>;

/// One list's current copy and how to read one.
struct Slot<T> {
    list: List,
    parse: fn(&[u8]) -> Result<T, String>,
    current: RwLock<Current<T>>,
}

impl<T> Slot<T> {
    const fn new(list: List, parse: fn(&[u8]) -> Result<T, String>) -> Self {
        Self {
            list,
            parse,
            current: RwLock::new(None),
        }
    }

    fn get(&self) -> (u64, Arc<T>) {
        if let Some(c) = &*self.current.read().unwrap_or_else(PoisonError::into_inner) {
            return c.clone();
        }
        let mut current = self.current.write().unwrap_or_else(PoisonError::into_inner);
        current
            .get_or_insert_with(|| {
                let parsed = (self.parse)(self.list.bundled()).unwrap_or_else(|e| {
                    panic!("the bundled {} is malformed: {e}", self.list.file())
                });
                (BUNDLED_SERIAL, Arc::new(parsed))
            })
            .clone()
    }

    fn serial(&self) -> u64 {
        match &*self.current.read().unwrap_or_else(PoisonError::into_inner) {
            Some((serial, _)) => *serial,
            None => BUNDLED_SERIAL,
        }
    }

    /// Makes `bytes` current if it is newer and well formed.
    fn set(&self, serial: u64, bytes: &[u8]) -> Result<bool, String> {
        if serial <= self.serial() {
            return Ok(false);
        }
        let parsed = (self.parse)(bytes)?;
        let mut current = self.current.write().unwrap_or_else(PoisonError::into_inner);
        if current.as_ref().is_some_and(|(s, _)| *s >= serial) {
            return Ok(false);
        }
        *current = Some((serial, Arc::new(parsed)));
        Ok(true)
    }
}

static COMPAT: Slot<Compat> = Slot::new(List::Compat, Compat::parse);
static SENSITIVE: Slot<SensitiveHosts> = Slot::new(List::SensitiveHosts, SensitiveHosts::parse);
static TOP: Slot<TopDomainList> = Slot::new(List::TopDomains, TopDomainList::parse);
static TRACKING: Slot<TrackingParams> = Slot::new(List::TrackingParams, TrackingParams::parse);
static HSTS: Slot<HstsPreload> = Slot::new(List::HstsPreload, HstsPreload::parse);
static FILTERS: Slot<FilterLists> = Slot::new(List::FilterLists, FilterLists::parse);
static PAYMENT: Slot<PaymentFrames> = Slot::new(List::PaymentFrames, PaymentFrames::parse);

pub fn payment_frames() -> Arc<PaymentFrames> {
    PAYMENT.get().1
}

pub fn filter_lists() -> Arc<FilterLists> {
    FILTERS.get().1
}

pub fn compat() -> Arc<Compat> {
    COMPAT.get().1
}

pub fn sensitive_hosts() -> Arc<SensitiveHosts> {
    SENSITIVE.get().1
}

pub fn top_domains() -> Arc<TopDomainList> {
    TOP.get().1
}

pub fn tracking_params() -> Arc<TrackingParams> {
    TRACKING.get().1
}

pub fn hsts_preload() -> Arc<HstsPreload> {
    HSTS.get().1
}

/// The current copy's serial: when it was signed, or [`BUNDLED_SERIAL`].
pub fn serial(list: List) -> u64 {
    match list {
        List::Compat => COMPAT.serial(),
        List::SensitiveHosts => SENSITIVE.serial(),
        List::TopDomains => TOP.serial(),
        List::TrackingParams => TRACKING.serial(),
        List::HstsPreload => HSTS.serial(),
        List::FilterLists => FILTERS.serial(),
        List::PaymentFrames => PAYMENT.serial(),
    }
}

/// Makes a verified copy current when it is newer than the current one.
pub(crate) fn set(list: List, serial: u64, bytes: &[u8]) -> Result<bool, String> {
    match list {
        List::Compat => COMPAT.set(serial, bytes),
        List::SensitiveHosts => SENSITIVE.set(serial, bytes),
        List::TopDomains => TOP.set(serial, bytes),
        List::TrackingParams => TRACKING.set(serial, bytes),
        List::HstsPreload => HSTS.set(serial, bytes),
        List::FilterLists => FILTERS.set(serial, bytes),
        List::PaymentFrames => PAYMENT.set(serial, bytes),
    }
}

pub(crate) fn parse_only(list: List, bytes: &[u8]) -> Result<(), String> {
    match list {
        List::Compat => Compat::parse(bytes).map(drop),
        List::SensitiveHosts => SensitiveHosts::parse(bytes).map(drop),
        List::TopDomains => TopDomainList::parse(bytes).map(drop),
        List::TrackingParams => TrackingParams::parse(bytes).map(drop),
        List::HstsPreload => HstsPreload::parse(bytes).map(drop),
        List::FilterLists => FilterLists::parse(bytes).map(drop),
        List::PaymentFrames => PaymentFrames::parse(bytes).map(drop),
    }
}

/// What a reader builds from a list (an index, a lookup table), rebuilt
/// the first time it is asked for after the list changes.
pub struct Derived<T> {
    list: List,
    build: fn() -> T,
    cell: LazyLock<RwLock<Current<T>>>,
}

impl<T> std::fmt::Debug for Derived<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Derived")
            .field("list", &self.list)
            .finish_non_exhaustive()
    }
}

impl<T> Derived<T> {
    /// `build` reads the list through its accessor.
    pub const fn new(list: List, build: fn() -> T) -> Self {
        Self {
            list,
            build,
            cell: LazyLock::new(|| RwLock::new(None)),
        }
    }

    pub fn get(&self) -> Arc<T> {
        let serial = serial(self.list);
        if let Some((s, v)) = &*self.cell.read().unwrap_or_else(PoisonError::into_inner)
            && *s == serial
        {
            return Arc::clone(v);
        }
        let built = Arc::new((self.build)());
        *self.cell.write().unwrap_or_else(PoisonError::into_inner) =
            Some((serial, Arc::clone(&built)));
        built
    }
}
