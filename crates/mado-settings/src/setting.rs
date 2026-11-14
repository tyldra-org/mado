//! The `settings!` macro (spec 06 §1) and what it generates: a
//! `SettingKey` variant, a marker type with a typed value per setting, and
//! one `SettingDef` in `REGISTRY`.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::types::{Restart, Scope, Section, SettingType};

/// Turns a stored value from an older `version` into the current shape;
/// `None` resets it to the default (with a log entry).
pub type Migrate = fn(stored_version: u32, value: &Value) -> Option<Value>;

/// The optional parts of a declaration; `Options::DEFAULT` fills the rest.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub restart: Restart,
    /// Only the settings window may change it (spec 06 §5).
    pub protected: bool,
    /// Mirrored into `state.settings.values`.
    pub ui: bool,
    pub version: u32,
    /// Hidden from the page and search while this feature is off.
    pub feature: Option<&'static str>,
    pub controllable_by_extension: bool,
    /// Changing it shows a 10-second notice with Undo.
    pub undo_notice: bool,
    pub migrate: Option<Migrate>,
}

impl Options {
    pub const DEFAULT: Options = Options {
        restart: Restart::None,
        protected: false,
        ui: true,
        version: 1,
        feature: None,
        controllable_by_extension: false,
        undo_notice: false,
        migrate: None,
    };
}

/// One registry entry.
#[derive(Debug, Clone, Copy)]
pub struct SettingDef {
    pub id: crate::SettingKey,
    pub key: &'static str,
    pub ty: SettingType,
    /// The default as JSON text.
    pub default_json: &'static str,
    pub scope: Scope,
    pub section: Section,
    pub label: &'static str,
    pub help: &'static str,
    pub keywords: &'static str,
    pub opts: Options,
}

impl SettingDef {
    pub fn default_value(&self) -> Value {
        // Every default is checked by a test; a broken one is a bug.
        serde_json::from_str(self.default_json).expect("registry defaults are valid JSON")
    }
}

/// A setting with a typed value: `settings.get::<TabsMemorySaver>()`.
pub trait Setting {
    type Value: DeserializeOwned;
    const KEY: crate::SettingKey;
}

macro_rules! settings {
    ($(
        $ident:ident : $rust:ty {
            key: $key:literal,
            ty: $ty:expr,
            default: $default:literal,
            scope: $scope:ident,
            section: $section:ident,
            $($field:ident : $val:expr,)*
        }
    )*) => {
        /// One variant per setting, in registry order.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum SettingKey {
            $($ident,)*
        }

        $(
            #[doc = concat!("`", $key, "`")]
            #[derive(Debug, Clone, Copy)]
            pub struct $ident;

            impl crate::setting::Setting for $ident {
                type Value = $rust;
                const KEY: SettingKey = SettingKey::$ident;
            }
        )*

        /// Every setting, in page order within each section.
        pub static REGISTRY: &[crate::setting::SettingDef] = &[$(
            crate::setting::SettingDef {
                id: SettingKey::$ident,
                key: $key,
                ty: $ty,
                default_json: $default,
                scope: crate::types::Scope::$scope,
                section: crate::types::Section::$section,
                label: concat!("settings.", $key, ".label"),
                help: concat!("settings.", $key, ".help"),
                keywords: concat!("settings.", $key, ".keywords"),
                opts: crate::setting::Options { $($field: $val,)* ..crate::setting::Options::DEFAULT },
            },
        )*];
    };
}

pub(crate) use settings;

/// The entry for `key`.
pub fn lookup(key: &str) -> Option<&'static SettingDef> {
    crate::REGISTRY.iter().find(|d| d.key == key)
}

/// The entry for a typed key.
pub fn def(id: crate::SettingKey) -> &'static SettingDef {
    // REGISTRY lists the variants in declaration order.
    &crate::REGISTRY[id as usize]
}
