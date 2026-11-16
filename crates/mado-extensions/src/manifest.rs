//! `manifest.json` (spec 09 §4): what Mado reads from it, its localised
//! strings, why a package is refused, and the permission summary the
//! install prompt shows.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

/// The permissions `WKWebExtension` grants on macOS 15.4
/// (`WKWebExtension.Permission`). Others are not available in Mado.
pub const SUPPORTED_PERMISSIONS: &[&str] = &[
    "activeTab",
    "alarms",
    "clipboardWrite",
    "contextMenus",
    "cookies",
    "declarativeNetRequest",
    "declarativeNetRequestFeedback",
    "declarativeNetRequestWithHostAccess",
    "menus",
    "nativeMessaging",
    "scripting",
    "storage",
    "tabs",
    "unlimitedStorage",
    "webNavigation",
    "webRequest",
];

/// What Mado reads from a manifest, strings localised.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub manifest_version: u64,
    pub description: String,
    /// Icon paths by size.
    pub icons: BTreeMap<u32, String>,
    /// Has a toolbar action (`action`, `browser_action` or `page_action`).
    pub has_action: bool,
    /// The action's popup page, if any.
    pub popup: Option<String>,
    pub permissions: Vec<String>,
    pub optional_permissions: Vec<String>,
    /// Match patterns: `host_permissions`, MV2 host entries in
    /// `permissions`, and content script matches.
    pub hosts: Vec<String>,
    pub optional_hosts: Vec<String>,
    /// `key`: the developer's public key, base64 DER, if present.
    pub key: Option<String>,
    pub update_url: Option<String>,
    pub incognito: Option<String>,
    pub options_page: Option<String>,
    /// `chrome_url_overrides.newtab`.
    pub new_tab_override: Option<String>,
    /// `chrome_settings_overrides` is present.
    pub settings_overrides: bool,
    /// Rule counts of `declarative_net_request.rule_resources`, unknown
    /// until the files are read (09 §8).
    pub rule_resources: Vec<String>,
    pub minimum_chrome_version: Option<String>,
}

/// Why a package cannot be installed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("themes are not available in Mado")]
    Theme,
    #[error("packaged apps are not available in Mado")]
    App,
    #[error("manifest version {0} is too old")]
    TooOld(u64),
    #[error("the manifest is malformed: {0}")]
    Malformed(String),
}

/// What the install prompt shows (09 §2.2 step 6, §4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    /// Required API permissions Mado supports.
    pub permissions: Vec<String>,
    /// Required host patterns.
    pub hosts: Vec<String>,
    /// It asks for every site.
    pub all_sites: bool,
    /// Optional API permissions and hosts it may ask for later, supported
    /// ones only.
    pub optional: Vec<String>,
    /// Permissions Mado does not support: optional ones are withheld,
    /// required ones may make features not work.
    pub unavailable: Vec<String>,
    /// A required permission is unavailable.
    pub missing_required: bool,
}

fn is_host_pattern(p: &str) -> bool {
    p == "<all_urls>" || p.contains("://")
}

/// Patterns that cover every site.
pub fn is_all_sites(p: &str) -> bool {
    matches!(
        p,
        "<all_urls>"
            | "*://*/*"
            | "http://*/*"
            | "https://*/*"
            | "*://*/"
            | "https://*/"
            | "http://*/"
    )
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// `_locales/<locale>/messages.json`, keys lowercased.
fn messages(dir: &Path, locale: &str) -> BTreeMap<String, String> {
    let path = dir.join("_locales").join(locale).join("messages.json");
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let text = text.trim_start_matches('\u{feff}');
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
        return BTreeMap::new();
    };
    map.into_iter()
        .filter_map(|(k, v)| Some((k.to_lowercase(), v.get("message")?.as_str()?.to_owned())))
        .collect()
}

/// Resolves `__MSG_name__` with the UI language, then `default_locale`.
struct Localiser {
    tables: Vec<BTreeMap<String, String>>,
}

impl Localiser {
    fn new(dir: &Path, language: &str, default_locale: Option<&str>) -> Self {
        let mut locales = vec![language.replace('-', "_")];
        if let Some((base, _)) = language.split_once(['-', '_']) {
            locales.push(base.to_owned());
        }
        locales.extend(default_locale.map(str::to_owned));
        Self {
            tables: locales.iter().map(|l| messages(dir, l)).collect(),
        }
    }

    fn get(&self, s: &str) -> String {
        let Some(key) = s.strip_prefix("__MSG_").and_then(|k| k.strip_suffix("__")) else {
            return s.to_owned();
        };
        let key = key.to_lowercase();
        self.tables
            .iter()
            .find_map(|t| t.get(&key))
            .cloned()
            .unwrap_or_else(|| s.to_owned())
    }
}

impl Manifest {
    /// Reads `<dir>/manifest.json`, localised for `language`.
    pub fn read(dir: &Path, language: &str) -> Result<Self, Refusal> {
        let text = std::fs::read_to_string(dir.join("manifest.json"))
            .map_err(|e| Refusal::Malformed(e.to_string()))?;
        Self::parse(text.trim_start_matches('\u{feff}'), dir, language)
    }

    fn parse(text: &str, dir: &Path, language: &str) -> Result<Self, Refusal> {
        let m: Value = serde_json::from_str(text).map_err(|e| Refusal::Malformed(e.to_string()))?;
        if !m.is_object() {
            return Err(Refusal::Malformed("not an object".into()));
        }
        if m.get("theme").is_some() {
            return Err(Refusal::Theme);
        }
        if m.get("app").is_some() {
            return Err(Refusal::App);
        }
        let manifest_version = m["manifest_version"].as_u64().unwrap_or(0);
        if manifest_version < 2 {
            return Err(Refusal::TooOld(manifest_version));
        }
        let l = Localiser::new(dir, language, m["default_locale"].as_str());
        let text_of = |v: &Value| v.as_str().map(|s| l.get(s));
        let name = text_of(&m["name"])
            .filter(|n| !n.trim().is_empty())
            .ok_or_else(|| Refusal::Malformed("no name".into()))?;
        let version = m["version"]
            .as_str()
            .filter(|v| {
                !v.is_empty()
                    && v.split('.').count() <= 4
                    && v.split('.')
                        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            })
            .ok_or_else(|| Refusal::Malformed("no valid version".into()))?
            .to_owned();
        let action = ["action", "browser_action", "page_action"]
            .iter()
            .find_map(|k| m.get(*k).filter(|v| v.is_object()));
        let popup = action
            .and_then(|a| a["default_popup"].as_str())
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        let icons = m["icons"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let (permissions, mut hosts): (Vec<String>, Vec<String>) = strings(&m["permissions"])
            .into_iter()
            .partition(|p| !is_host_pattern(p));
        let (optional_permissions, mut optional_hosts): (Vec<String>, Vec<String>) =
            strings(&m["optional_permissions"])
                .into_iter()
                .partition(|p| !is_host_pattern(p));
        hosts.extend(strings(&m["host_permissions"]));
        optional_hosts.extend(strings(&m["optional_host_permissions"]));
        for script in m["content_scripts"].as_array().into_iter().flatten() {
            hosts.extend(strings(&script["matches"]));
        }
        hosts.sort();
        hosts.dedup();
        Ok(Self {
            name,
            version,
            manifest_version,
            description: text_of(&m["description"]).unwrap_or_default(),
            icons,
            has_action: action.is_some(),
            popup,
            permissions,
            optional_permissions,
            hosts,
            optional_hosts,
            key: m["key"].as_str().map(str::to_owned),
            update_url: m["update_url"].as_str().map(str::to_owned),
            incognito: m["incognito"].as_str().map(str::to_owned),
            options_page: m["options_page"]
                .as_str()
                .or_else(|| m["options_ui"]["page"].as_str())
                .map(str::to_owned),
            new_tab_override: m["chrome_url_overrides"]["newtab"]
                .as_str()
                .map(str::to_owned),
            settings_overrides: m.get("chrome_settings_overrides").is_some(),
            rule_resources: m["declarative_net_request"]["rule_resources"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r["path"].as_str().map(str::to_owned))
                .collect(),
            minimum_chrome_version: m["minimum_chrome_version"].as_str().map(str::to_owned),
        })
    }

    /// The largest icon, as a path inside the package.
    pub fn icon(&self) -> Option<&str> {
        self.icons.iter().next_back().map(|(_, p)| p.as_str())
    }

    /// The permission summary with `supported` API permissions.
    pub fn summary(&self, supported: &[&str]) -> Summary {
        let ok = |p: &String| supported.contains(&p.as_str());
        let unavailable: Vec<String> = self
            .permissions
            .iter()
            .chain(&self.optional_permissions)
            .filter(|p| !ok(p))
            .cloned()
            .collect();
        Summary {
            permissions: self.permissions.iter().filter(|p| ok(p)).cloned().collect(),
            hosts: self.hosts.clone(),
            all_sites: self.hosts.iter().any(|h| is_all_sites(h)),
            optional: self
                .optional_permissions
                .iter()
                .filter(|p| ok(p))
                .chain(&self.optional_hosts)
                .cloned()
                .collect(),
            missing_required: self.permissions.iter().any(|p| !ok(p)),
            unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Manifest, Refusal> {
        Manifest::parse(json, Path::new("/nonexistent"), "en")
    }

    #[test]
    fn themes_apps_and_old_formats_are_refused() {
        assert_eq!(
            parse(r#"{"name":"t","version":"1","manifest_version":3,"theme":{}}"#),
            Err(Refusal::Theme)
        );
        assert_eq!(
            parse(r#"{"name":"a","version":"1","manifest_version":2,"app":{}}"#),
            Err(Refusal::App)
        );
        assert_eq!(
            parse(r#"{"name":"o","version":"1","manifest_version":1}"#),
            Err(Refusal::TooOld(1))
        );
        assert!(matches!(
            parse(r#"{"version":"1","manifest_version":3}"#),
            Err(Refusal::Malformed(_))
        ));
        assert!(matches!(
            parse(r#"{"name":"x","version":"1.a","manifest_version":3}"#),
            Err(Refusal::Malformed(_))
        ));
    }

    #[test]
    fn the_summary_splits_hosts_and_withholds_what_mado_lacks() {
        let m = parse(
            r#"{
            "name": "x", "version": "1.2.3", "manifest_version": 2,
            "browser_action": { "default_popup": "popup.html" },
            "permissions": ["storage", "tabs", "https://a.example/*", "history"],
            "optional_permissions": ["bookmarks", "cookies", "https://b.example/*"],
            "content_scripts": [{ "matches": ["<all_urls>"], "js": ["c.js"] }],
            "icons": { "16": "i16.png", "128": "i128.png" }
        }"#,
        )
        .expect("parses");
        assert_eq!(m.popup.as_deref(), Some("popup.html"));
        assert_eq!(m.icon(), Some("i128.png"));
        let s = m.summary(SUPPORTED_PERMISSIONS);
        assert_eq!(s.permissions, ["storage", "tabs"]);
        assert_eq!(s.hosts, ["<all_urls>", "https://a.example/*"]);
        assert!(s.all_sites);
        assert_eq!(s.optional, ["cookies", "https://b.example/*"]);
        assert_eq!(s.unavailable, ["history", "bookmarks"]);
        assert!(s.missing_required);
    }

    #[test]
    fn names_resolve_with_the_language_then_the_default_locale() {
        let dir = tempfile::tempdir().expect("dir");
        let write = |locale: &str, json: &str| {
            let d = dir.path().join("_locales").join(locale);
            std::fs::create_dir_all(&d).expect("dir");
            std::fs::write(d.join("messages.json"), json).expect("write");
        };
        write("fr", r#"{"extName":{"message":"Bloqueur"}}"#);
        write(
            "en",
            r#"{"EXTNAME":{"message":"Blocker"},"desc":{"message":"Blocks"}}"#,
        );
        let json = r#"{"name":"__MSG_extName__","description":"__MSG_desc__","version":"1","manifest_version":3,"default_locale":"en"}"#;
        let fr = Manifest::parse(json, dir.path(), "fr-CA").expect("fr");
        assert_eq!(
            (fr.name.as_str(), fr.description.as_str()),
            ("Bloqueur", "Blocks")
        );
        let de = Manifest::parse(json, dir.path(), "de").expect("de");
        assert_eq!(de.name, "Blocker");
    }
}
