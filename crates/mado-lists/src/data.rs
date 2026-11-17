//! What each list holds. Readers ignore fields they do not know, so a list
//! can grow without breaking older copies of Mado; checks reject what
//! would be wrong for every version.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;

use serde::Deserialize;

/// `compat.json` (spec 09 §7).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compat {
    /// By Chrome extension ID.
    #[serde(default)]
    pub extensions: BTreeMap<String, CompatEntry>,
    /// Extensions disabled wherever they are installed, by ID.
    #[serde(default)]
    pub blocked: BTreeMap<String, Blocked>,
    /// Sites whose notification requests are refused without asking.
    #[serde(default)]
    pub crowd_deny_notifications: Vec<String>,
    pub min_safe_engine: MinSafeEngine,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompatEntry {
    pub name: String,
    pub status: Status,
    pub category: Category,
    /// The password manager, for `passwordManager` entries.
    #[serde(default)]
    pub manager: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Tested,
    Mostly,
    /// Also any status a newer list adds.
    #[serde(other)]
    NotSupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    AdBlocker,
    PasswordManager,
    DarkMode,
    Translation,
    TabSleeping,
    Theme,
    /// Also any category a newer list adds.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Blocked {
    pub reason: String,
    pub since: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MinSafeEngine {
    /// The oldest WebKit without known security fixes pending (15 §5).
    pub webkit: String,
}

/// `filter-lists.json` (10 §2): the blocker's lists and resources.
#[derive(Debug, Clone, Deserialize)]
pub struct FilterLists {
    pub lists: Vec<FilterList>,
    /// URLs of JSON arrays of adblock-rust resources (scriptlets and
    /// redirects), fetched at runtime.
    #[serde(default)]
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FilterList {
    /// Lowercase letters, digits and `-`.
    pub id: String,
    pub url: String,
    /// On unless the user's `blocker.lists` says otherwise.
    #[serde(default)]
    pub default: bool,
    pub kind: ListKind,
}

/// What a list blocks, for the statistics (10 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListKind {
    Ads,
    Trackers,
    Annoyances,
    /// Also any kind a newer list adds.
    #[serde(other)]
    Other,
}

impl FilterList {
    /// `ads`, `trackers`, `annoyances` or `other`.
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            ListKind::Ads => "ads",
            ListKind::Trackers => "trackers",
            ListKind::Annoyances => "annoyances",
            ListKind::Other => "other",
        }
    }
}

impl FilterLists {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let list: Self = json(bytes)?;
        if list.lists.is_empty() {
            return Err("no lists".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for l in &list.lists {
            if l.id.is_empty()
                || !l
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                return Err(format!("{:?} is not a list ID", l.id));
            }
            if !seen.insert(l.id.as_str()) {
                return Err(format!("{} appears twice", l.id));
            }
            if !l.url.starts_with("https://") {
                return Err(format!("{}: not an https URL", l.id));
            }
        }
        if let Some(r) = list.resources.iter().find(|r| !r.starts_with("https://")) {
            return Err(format!("resources: {r:?} is not an https URL"));
        }
        Ok(list)
    }
}

/// `sensitive-hosts.json` (12 §2.2, 13 §9).
#[derive(Debug, Clone, Deserialize)]
pub struct SensitiveHosts {
    /// Banks; a host matches itself and its subdomains.
    pub banking: Vec<String>,
}

/// `models.json` (13 §4): known models by `provider/model`, and the
/// subscription CLIs' commands (13 §2.3, M5-T13).
#[derive(Debug, Clone, Deserialize)]
pub struct Models {
    pub models: BTreeMap<String, ModelInfo>,
    #[serde(default)]
    pub cli: BTreeMap<String, serde_json::Value>,
}

/// What Mado knows about one model.
#[derive(Debug, Clone, PartialEq, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub name: String,
    pub context_tokens: u64,
    pub max_output: u64,
    pub vision: bool,
    pub tools: bool,
    pub reasoning: bool,
    pub price_per_m_tok: Price,
    /// Role to rank, 1 the best.
    #[serde(default)]
    pub roles: BTreeMap<String, u32>,
}

/// US dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cached_input: f64,
}

/// The roles of 13 §4.
pub const ROLES: [&str; 7] = [
    "chat", "tasks", "fast", "deep", "vision", "commands", "image",
];

impl Models {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let list: Self = json(bytes)?;
        if list.models.is_empty() {
            return Err("models is empty".into());
        }
        for (key, m) in &list.models {
            if key
                .split_once('/')
                .is_none_or(|(p, id)| p.is_empty() || id.is_empty())
            {
                return Err(format!("{key} is not provider/model"));
            }
            if let Some(role) = m.roles.keys().find(|r| !ROLES.contains(&r.as_str())) {
                return Err(format!("{key}: unknown role {role}"));
            }
            let p = m.price_per_m_tok;
            if [p.input, p.output, p.cached_input]
                .iter()
                .any(|x| !x.is_finite() || *x < 0.0)
            {
                return Err(format!("{key}: a price is not a positive number"));
            }
        }
        Ok(list)
    }
}

/// `payment-frames.json` (11 §4.1): hosts whose frames may get card
/// fills; a host matches itself only.
#[derive(Debug, Clone, Deserialize)]
pub struct PaymentFrames {
    pub hosts: Vec<String>,
}

impl PaymentFrames {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let list: Self = json(bytes)?;
        if list.hosts.is_empty() {
            return Err("hosts is empty".into());
        }
        hosts_ok("hosts", &list.hosts)?;
        Ok(list)
    }

    /// Whether a frame at `origin` (`https://host[:port]`) is a payment
    /// frame. Only https counts.
    pub fn allows(&self, origin: &str) -> bool {
        origin
            .strip_prefix("https://")
            .map(|h| h.split(':').next().unwrap_or(h))
            .is_some_and(|h| self.hosts.iter().any(|x| x == h))
    }
}

/// `tracking-params.json` (08 §8).
#[derive(Debug, Clone, Deserialize)]
pub struct TrackingParams {
    /// Any parameter starting with one of these.
    pub prefixes: Vec<String>,
    /// These names on every host.
    pub params: Vec<String>,
    /// These names on the host and its subdomains only.
    #[serde(default)]
    pub hosts: BTreeMap<String, Vec<String>>,
}

/// `top-domains.txt`: one domain per line, `#` starts a comment line.
#[derive(Debug, Clone)]
pub struct TopDomainList {
    pub domains: Vec<String>,
}

/// `hsts-preload.txt.gz`: gzipped lines of `host`, then `+` when its
/// subdomains are included; `#` starts a comment line.
#[derive(Debug, Clone)]
pub struct HstsPreload {
    /// Host to whether its subdomains are included.
    pub hosts: HashMap<String, bool>,
}

impl HstsPreload {
    /// Whether `host` is on the list, itself or under an entry that
    /// includes subdomains.
    pub fn preloaded(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if self.hosts.contains_key(&host) {
            return true;
        }
        let mut rest = host.as_str();
        while let Some((_, parent)) = rest.split_once('.') {
            if self.hosts.get(parent) == Some(&true) {
                return true;
            }
            rest = parent;
        }
        false
    }
}

/// A lowercase host name: letters, digits, `-`, `_` and inner dots.
fn is_host(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && !s.ends_with('.')
        && !s.contains("..")
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
}

fn hosts_ok<'a>(what: &str, hosts: impl IntoIterator<Item = &'a String>) -> Result<(), String> {
    match hosts.into_iter().find(|h| !is_host(h)) {
        Some(bad) => Err(format!("{what}: {bad:?} is not a lowercase host")),
        None => Ok(()),
    }
}

fn json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}

/// A Chrome extension ID: 32 letters from a to p.
fn is_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

impl Compat {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let list: Self = json(bytes)?;
        if let Some(id) = list
            .extensions
            .keys()
            .chain(list.blocked.keys())
            .find(|id| !is_extension_id(id))
        {
            return Err(format!("{id:?} is not an extension ID"));
        }
        let engine = &list.min_safe_engine.webkit;
        if engine.is_empty()
            || !engine
                .split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(format!("minSafeEngine.webkit {engine:?} is not a version"));
        }
        hosts_ok("crowdDenyNotifications", &list.crowd_deny_notifications)?;
        Ok(list)
    }
}

impl SensitiveHosts {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let list: Self = json(bytes)?;
        if list.banking.is_empty() {
            return Err("banking is empty".into());
        }
        hosts_ok("banking", &list.banking)?;
        Ok(list)
    }
}

impl TrackingParams {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let list: Self = json(bytes)?;
        if list.params.is_empty() {
            return Err("params is empty".into());
        }
        if list
            .prefixes
            .iter()
            .chain(&list.params)
            .chain(list.hosts.values().flatten())
            .any(String::is_empty)
        {
            return Err("an empty parameter name".into());
        }
        hosts_ok("hosts", list.hosts.keys())?;
        Ok(list)
    }
}

/// Lines that are not empty or comments.
fn entries(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
}

impl TopDomainList {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        let domains: Vec<String> = entries(text).map(str::to_owned).collect();
        if domains.is_empty() {
            return Err("no domains".into());
        }
        hosts_ok("domains", &domains)?;
        Ok(Self { domains })
    }
}

impl HstsPreload {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut text = String::new();
        flate2::read::GzDecoder::new(bytes)
            .read_to_string(&mut text)
            .map_err(|e| format!("gzip: {e}"))?;
        let mut hosts = HashMap::new();
        for line in entries(&text) {
            let mut parts = line.split(' ');
            let host = parts.next().unwrap_or_default();
            let subdomains = match parts.next() {
                None => false,
                Some("+") => true,
                Some(other) => return Err(format!("{host}: unknown flag {other:?}")),
            };
            if !is_host(host) {
                return Err(format!("{host:?} is not a lowercase host"));
            }
            hosts.insert(host.to_owned(), subdomains);
        }
        if hosts.is_empty() {
            return Err("no hosts".into());
        }
        Ok(Self { hosts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_reject_what_no_version_could_use() {
        let compat = |s: &str| Compat::parse(s.as_bytes()).map(|_| ());
        assert!(compat(r#"{"minSafeEngine":{"webkit":"620.1.15"}}"#).is_ok());
        assert!(compat(r#"{"minSafeEngine":{"webkit":"620.x"}}"#).is_err());
        assert!(compat(r#"{"extensions":{"abc":{"name":"x","status":"tested","category":"other"}},"minSafeEngine":{"webkit":"1"}}"#).is_err());
        let newer = r#"{"extensions":{"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":{"name":"x","status":"retired","category":"vpn"}},"later":1,"minSafeEngine":{"webkit":"1"}}"#;
        let list = Compat::parse(newer.as_bytes()).expect("a newer list still reads");
        let entry = &list.extensions["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"];
        assert_eq!(
            (entry.status, entry.category),
            (Status::NotSupported, Category::Other)
        );
        assert!(SensitiveHosts::parse(br#"{"banking":["Chase.com"]}"#).is_err());
        assert!(TopDomainList::parse(b"# only a comment\n").is_err());
    }

    #[test]
    fn hsts_entries_cover_subdomains_only_when_marked() {
        let list = HstsPreload {
            hosts: [("bank".to_owned(), true), ("example.com".to_owned(), false)].into(),
        };
        assert!(list.preloaded("my.bank"));
        assert!(list.preloaded("Example.com."));
        assert!(!list.preloaded("www.example.com"));
    }
}
