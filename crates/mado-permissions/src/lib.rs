//! Site permission store, prompt queue and auto-revoke.
//!
//! Specification: `mado-docs/spec/07-permissions.md`. Roadmap: M2-T07.
//! Pure rules: the core loads `site_settings` rows into a [`Store`],
//! decides with it, and writes changes back.

mod applinks;

pub use applinks::{LaunchLimiter, blocked_scheme};

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

/// A site setting's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Value {
    Allow,
    Block,
    Ask,
}

impl Value {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Block => "block",
            Self::Ask => "ask",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "allow" => Some(Self::Allow),
            "block" => Some(Self::Block),
            "ask" => Some(Self::Ask),
            _ => None,
        }
    }
}

/// Who wrote a row (`site_settings.source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    User,
    /// Written by Mado itself: three dismissals ("Blocked automatically").
    Default,
    Extension,
    /// Unused for 60 days; treated as `ask`.
    AutoRevoked,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Default => "default",
            Self::Extension => "extension",
            Self::AutoRevoked => "auto_revoked",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "default" => Some(Self::Default),
            "extension" => Some(Self::Extension),
            "auto_revoked" => Some(Self::AutoRevoked),
            _ => None,
        }
    }
}

/// One `site_settings` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// `scheme://host[:port]` or a pattern `[*.]host`.
    pub origin: String,
    pub key: String,
    pub value: Value,
    /// The top-level origin for embedded requests; empty for any.
    pub embedder: String,
    pub last_used_at: Option<i64>,
    pub granted_at: i64,
    pub expires_at: Option<i64>,
    pub source: Source,
}

/// Permission keys whose `allow` is revoked after 60 unused days (07 §5).
pub const REVOCABLE: &[&str] = &[
    "camera",
    "microphone",
    "location",
    "notifications",
    "clipboardRead",
    "midi",
    "windowManagement",
];

/// Keys the WebKit engine cannot honour, hidden from Site settings (07 §1).
pub const HIDDEN_ON_WEBKIT: &[&str] = &[
    "midi",
    "idleDetection",
    "localNetwork",
    "windowManagement",
    "motionSensors",
];

const DAY: i64 = 86_400_000;

/// `scheme://host[:port]` of a URL, the origin site settings use.
pub fn origin_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?;
    Some(match u.port() {
        Some(p) => format!("{}://{host}:{p}", u.scheme()),
        None => format!("{}://{host}", u.scheme()),
    })
}

/// Whether a `[*.]host` pattern covers `origin`.
pub fn pattern_matches(pattern: &str, origin: &str) -> bool {
    let Some(base) = pattern.strip_prefix("[*.]") else {
        return false;
    };
    let host = url::Url::parse(origin)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    let base = base.to_ascii_lowercase();
    host == base || host.ends_with(&format!(".{base}"))
}

/// The profile's site settings, in memory.
#[derive(Debug, Clone, Default)]
pub struct Store {
    rows: BTreeMap<(String, String, String), Row>,
}

impl Store {
    pub fn new(rows: impl IntoIterator<Item = Row>) -> Self {
        Self {
            rows: rows
                .into_iter()
                .map(|r| ((r.origin.clone(), r.key.clone(), r.embedder.clone()), r))
                .collect(),
        }
    }

    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.values()
    }

    fn live(row: &Row, now: i64) -> Option<Value> {
        if row.expires_at.is_some_and(|e| e <= now) {
            return None;
        }
        Some(if row.source == Source::AutoRevoked {
            Value::Ask
        } else {
            row.value
        })
    }

    /// 07 §2 step 3: the exact origin with this embedder, then any
    /// embedder, then patterns, then `default`.
    pub fn decide(
        &self,
        origin: &str,
        key: &str,
        embedder: Option<&str>,
        default: Value,
        now: i64,
    ) -> Value {
        let exact = |emb: &str| {
            self.rows
                .get(&(origin.to_owned(), key.to_owned(), emb.to_owned()))
                .and_then(|r| Self::live(r, now))
        };
        if let Some(e) = embedder
            && let Some(v) = exact(e)
        {
            return v;
        }
        if let Some(v) = exact("") {
            return v;
        }
        self.rows
            .values()
            .filter(|r| r.key == key && r.embedder.is_empty() && pattern_matches(&r.origin, origin))
            .find_map(|r| Self::live(r, now))
            .unwrap_or(default)
    }

    /// The row a decision came from, for `last_used_at`.
    pub fn touch(&mut self, origin: &str, key: &str, embedder: &str, now: i64) -> Option<Row> {
        let row = self
            .rows
            .get_mut(&(origin.to_owned(), key.to_owned(), embedder.to_owned()))?;
        row.last_used_at = Some(now);
        Some(row.clone())
    }

    pub fn set(&mut self, row: Row) {
        self.rows.insert(
            (row.origin.clone(), row.key.clone(), row.embedder.clone()),
            row,
        );
    }

    /// Removes `origin`'s rows for `key`, or all of them. Returns them.
    pub fn remove(&mut self, origin: &str, key: Option<&str>) -> Vec<Row> {
        let gone: Vec<(String, String, String)> = self
            .rows
            .keys()
            .filter(|(o, k, _)| o == origin && key.is_none_or(|key| key == k))
            .cloned()
            .collect();
        gone.into_iter()
            .filter_map(|k| self.rows.remove(&k))
            .collect()
    }

    /// 07 §5: `allow` rows of revocable keys unused for 60 days whose
    /// origin had no visit in 60 days. Marks them and returns them.
    pub fn auto_revoke(&mut self, now: i64, visited_recently: impl Fn(&str) -> bool) -> Vec<Row> {
        let mut out = Vec::new();
        for row in self.rows.values_mut() {
            let used = row.last_used_at.unwrap_or(row.granted_at);
            if row.value == Value::Allow
                && row.source == Source::User
                && row.expires_at.is_none_or(|e| e > now)
                && REVOCABLE.contains(&row.key.as_str())
                && now - used > 60 * DAY
                && !visited_recently(&row.origin)
            {
                row.source = Source::AutoRevoked;
                out.push(row.clone());
            }
        }
        out
    }
}

/// Prompt dismissals per origin and key (07 §3): the third within 7 days
/// blocks.
#[derive(Debug, Clone, Default)]
pub struct Dismissals {
    seen: HashMap<(String, String), Vec<i64>>,
}

impl Dismissals {
    /// Records one; true when this one blocks the site.
    pub fn record(&mut self, origin: &str, key: &str, now: i64) -> bool {
        let list = self
            .seen
            .entry((origin.to_owned(), key.to_owned()))
            .or_default();
        list.retain(|at| now - at < 7 * DAY);
        list.push(now);
        list.len() >= 3
    }

    /// Times dismissed in the last 7 days (quiet mode for notifications).
    pub fn count(&self, origin: &str, key: &str, now: i64) -> usize {
        self.seen
            .get(&(origin.to_owned(), key.to_owned()))
            .map_or(0, |l| l.iter().filter(|at| now - **at < 7 * DAY).count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(origin: &str, key: &str, value: Value, embedder: &str) -> Row {
        Row {
            origin: origin.into(),
            key: key.into(),
            value,
            embedder: embedder.into(),
            last_used_at: None,
            granted_at: 0,
            expires_at: None,
            source: Source::User,
        }
    }

    #[test]
    fn lookups_go_embedder_then_origin_then_pattern_then_default() {
        let s = Store::new([
            row("https://a.example", "camera", Value::Allow, ""),
            row(
                "https://a.example",
                "camera",
                Value::Block,
                "https://top.example",
            ),
            row("[*.]b.example", "camera", Value::Allow, ""),
        ]);
        let d = |o: &str, e: Option<&str>| s.decide(o, "camera", e, Value::Ask, 0);
        assert_eq!(d("https://a.example", None), Value::Allow);
        assert_eq!(
            d("https://a.example", Some("https://top.example")),
            Value::Block
        );
        assert_eq!(
            d("https://a.example", Some("https://other.example")),
            Value::Allow
        );
        assert_eq!(d("https://x.b.example", None), Value::Allow);
        assert_eq!(d("https://c.example", None), Value::Ask);
    }

    #[test]
    fn expired_and_revoked_rows_fall_back() {
        let mut s = Store::new([Row {
            expires_at: Some(10),
            ..row("https://a.example", "camera", Value::Allow, "")
        }]);
        assert_eq!(
            s.decide("https://a.example", "camera", None, Value::Ask, 11),
            Value::Ask
        );
        s.set(row("https://m.example", "microphone", Value::Allow, ""));
        let gone = s.auto_revoke(61 * DAY, |_| false);
        assert_eq!(gone.len(), 1);
        assert_eq!(
            s.decide(
                "https://m.example",
                "microphone",
                None,
                Value::Block,
                61 * DAY
            ),
            Value::Ask
        );
        let mut kept = Store::new([row("https://v.example", "camera", Value::Allow, "")]);
        assert!(kept.auto_revoke(61 * DAY, |_| true).is_empty());
    }

    #[test]
    fn three_dismissals_in_a_week_block() {
        let mut d = Dismissals::default();
        assert!(!d.record("https://a.example", "notifications", 0));
        assert!(!d.record("https://a.example", "notifications", DAY));
        assert!(d.record("https://a.example", "notifications", 2 * DAY));
        let mut late = Dismissals::default();
        late.record("o", "k", 0);
        late.record("o", "k", 8 * DAY);
        assert!(!late.record("o", "k", 9 * DAY));
    }

    #[test]
    fn origins_keep_their_port() {
        assert_eq!(
            origin_of("https://a.example:8443/x").as_deref(),
            Some("https://a.example:8443")
        );
        assert_eq!(
            origin_of("https://a.example/x").as_deref(),
            Some("https://a.example")
        );
        assert!(origin_of("about:blank").is_none());
    }
}
