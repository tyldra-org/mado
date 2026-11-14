//! `en.json` strings for labels, help and search keywords. Rust never shows
//! them; it searches them (spec 06 §3) and checks they exist.

use std::collections::BTreeMap;

const EN: &str = include_str!("../../../ui/src/i18n/en.json");

/// A flat `key -> ICU message` map.
#[derive(Debug, Clone, Default)]
pub struct Strings(BTreeMap<String, String>);

impl Strings {
    /// The English strings built into this binary.
    pub fn english() -> Self {
        Self::parse(EN).expect("en.json is a flat object of strings")
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json).map(Self)
    }

    /// The message, or the key itself when it is missing.
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.0.get(key).map_or(key, String::as_str)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
}
