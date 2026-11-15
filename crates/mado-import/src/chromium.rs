//! Chrome, Edge, Brave and Arc: one user data folder with a `Local State`
//! and a folder per profile (spec 12 §5).

use std::path::Path;

use serde_json::Value;

use crate::{Bookmark, Browser, Engine, Extension, ImportError, Imported, Kind, Source, Visit};

fn json(path: &Path) -> Result<Value, ImportError> {
    let text = std::fs::read_to_string(path).map_err(|e| ImportError::io(path, e))?;
    serde_json::from_str(&text).map_err(|e| ImportError::format(path, e.to_string()))
}

/// The profiles `Local State` lists, with their display names.
pub fn profiles(browser: Browser, dir: &Path) -> Vec<Source> {
    let Ok(state) = json(&dir.join("Local State")) else {
        return Vec::new();
    };
    let Some(cache) = state["profile"]["info_cache"].as_object() else {
        return Vec::new();
    };
    let mut out: Vec<Source> = cache
        .iter()
        .filter(|(folder, _)| dir.join(folder).is_dir())
        .map(|(folder, info)| Source {
            browser,
            profile: folder.clone(),
            name: info["name"].as_str().unwrap_or(folder).to_owned(),
            path: dir.join(folder),
        })
        .collect();
    out.sort_by(|a, b| a.profile.cmp(&b.profile));
    out
}

pub fn read(profile: &Path, kinds: &[Kind], since: Option<i64>) -> Result<Imported, ImportError> {
    let mut out = Imported::default();
    if kinds.contains(&Kind::Bookmarks) {
        let path = profile.join("Bookmarks");
        if path.exists() {
            let b = json(&path)?;
            out.bar = children(&b["roots"]["bookmark_bar"]);
            out.other = children(&b["roots"]["other"]);
        }
    }
    if kinds.contains(&Kind::History) && profile.join("History").exists() {
        out.visits = history(&profile.join("History"), since)?;
    }
    if kinds.contains(&Kind::SearchEngines) {
        if profile.join("Web Data").exists() {
            out.engines = engines(&profile.join("Web Data"))?;
        }
        if let Ok(prefs) = json(&profile.join("Preferences")) {
            out.default_engine =
                prefs["default_search_provider_data"]["template_url_data"]["short_name"]
                    .as_str()
                    .map(str::to_owned);
        }
    }
    if kinds.contains(&Kind::Tabs)
        && let Some(path) = latest_session(&profile.join("Sessions"))
    {
        let bytes = std::fs::read(&path).map_err(|e| ImportError::io(&path, e))?;
        out.windows = crate::snss::parse(&bytes).map_err(|e| ImportError::format(&path, e))?;
    }
    if kinds.contains(&Kind::Extensions) {
        out.extensions = extensions(profile);
    }
    Ok(out)
}

fn children(node: &Value) -> Vec<Bookmark> {
    node["children"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|n| {
                    let title = n["name"].as_str().unwrap_or_default().to_owned();
                    match n["type"].as_str() {
                        Some("url") => n["url"].as_str().map(|u| Bookmark::link(title, u)),
                        Some("folder") => Some(Bookmark::folder(title, children(n))),
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn history(path: &Path, since: Option<i64>) -> Result<Vec<Visit>, ImportError> {
    let (conn, _dir) = crate::sqlite::open_copy(path)?;
    let from = since.map_or(i64::MIN, |ms| (ms + 11_644_473_600_000) * 1000);
    let sql = |e| ImportError::Sqlite(path.to_owned(), e);
    let mut stmt = conn
        .prepare(
            "SELECT u.url, COALESCE(u.title, ''), v.visit_time FROM visits v JOIN urls u ON u.id = v.url
             WHERE v.visit_time >= ?1 ORDER BY v.visit_time",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map([from], |r| {
            Ok(Visit {
                url: r.get(0)?,
                title: r.get(1)?,
                at: crate::from_chromium(r.get(2)?),
            })
        })
        .map_err(sql)?;
    rows.collect::<Result<_, _>>().map_err(sql)
}

fn engines(path: &Path) -> Result<Vec<Engine>, ImportError> {
    let (conn, _dir) = crate::sqlite::open_copy(path)?;
    let sql = |e| ImportError::Sqlite(path.to_owned(), e);
    let mut stmt = conn
        .prepare("SELECT short_name, keyword, url FROM keywords ORDER BY id")
        .map_err(sql)?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Engine {
                name: r.get(0)?,
                keyword: r.get(1)?,
                search_url: r.get(2)?,
            })
        })
        .map_err(sql)?;
    let all: Vec<Engine> = rows.collect::<Result<_, _>>().map_err(sql)?;
    Ok(all
        .into_iter()
        .filter(|e| {
            e.search_url.contains("{searchTerms}")
                && (e.search_url.starts_with("https://") || e.search_url.starts_with("http://"))
        })
        .collect())
}

/// The newest `Session_<time>` file, or the older `Current Session`.
fn latest_session(dir: &Path) -> Option<std::path::PathBuf> {
    let mut sessions: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("Session_"))
        })
        .collect();
    sessions.sort();
    sessions
        .pop()
        .or_else(|| Some(dir.join("Current Session")).filter(|p| p.exists()))
}

/// Web Store extensions from `Preferences` and `Secure Preferences`.
fn extensions(profile: &Path) -> Vec<Extension> {
    let mut out: Vec<Extension> = ["Preferences", "Secure Preferences"]
        .iter()
        .filter_map(|f| json(&profile.join(f)).ok())
        .flat_map(|p| {
            p["extensions"]["settings"]
                .as_object()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .filter(|(id, s)| {
            s["from_webstore"].as_bool() == Some(true)
                && id.len() == 32
                && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
        })
        .map(|(id, s)| Extension {
            name: s["manifest"]["name"].as_str().unwrap_or(&id).to_owned(),
            id,
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    out
}
