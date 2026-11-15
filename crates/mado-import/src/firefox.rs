//! Firefox: profiles from `profiles.ini`, `places.sqlite` for bookmarks
//! and history, mozLz4 files for search engines and the session (spec 12
//! §5).

use std::path::Path;

use serde_json::Value;

use crate::{Bookmark, Browser, Engine, ImportError, Imported, Kind, Source, Tab, Visit};

/// `moz_bookmarks` rows: id, parent, type, title, URL, GUID.
type Row = (i64, i64, i64, String, Option<String>, String);

/// The profiles `profiles.ini` lists that exist.
pub fn profiles(dir: &Path) -> Vec<Source> {
    let Ok(ini) = std::fs::read_to_string(dir.join("profiles.ini")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut name = None;
    let mut path = None;
    let mut relative = true;
    let mut flush = |name: &mut Option<String>, path: &mut Option<String>, relative: bool| {
        if let (Some(n), Some(p)) = (name.take(), path.take()) {
            let full = if relative {
                dir.join(&p)
            } else {
                p.clone().into()
            };
            if full.join("places.sqlite").exists() {
                out.push(Source {
                    browser: Browser::Firefox,
                    profile: p,
                    name: n,
                    path: full,
                });
            }
        }
    };
    for line in ini.lines().map(str::trim) {
        if line.starts_with('[') {
            flush(&mut name, &mut path, relative);
            relative = true;
        } else if let Some(v) = line.strip_prefix("Name=") {
            name = Some(v.to_owned());
        } else if let Some(v) = line.strip_prefix("Path=") {
            path = Some(v.to_owned());
        } else if let Some(v) = line.strip_prefix("IsRelative=") {
            relative = v != "0";
        }
    }
    flush(&mut name, &mut path, relative);
    out
}

/// A mozLz4 file: `mozLz40\0`, a `u32` size, an LZ4 block of JSON.
pub fn mozlz4(path: &Path) -> Result<Value, ImportError> {
    let bytes = std::fs::read(path).map_err(|e| ImportError::io(path, e))?;
    let body = bytes
        .strip_prefix(b"mozLz40\0")
        .ok_or_else(|| ImportError::format(path, "not a mozLz4 file"))?;
    let size = body
        .get(..4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| ImportError::format(path, "no size"))?;
    let json = lz4_flex::block::decompress(&body[4..], usize::try_from(size).unwrap_or(0))
        .map_err(|e| ImportError::format(path, e.to_string()))?;
    serde_json::from_slice(&json).map_err(|e| ImportError::format(path, e.to_string()))
}

pub fn read(profile: &Path, kinds: &[Kind], since: Option<i64>) -> Result<Imported, ImportError> {
    let mut out = Imported::default();
    let places = profile.join("places.sqlite");
    if (kinds.contains(&Kind::Bookmarks) || kinds.contains(&Kind::History)) && places.exists() {
        let (conn, _dir) = crate::sqlite::open_copy(&places)?;
        let sql = |e| ImportError::Sqlite(places.clone(), e);
        if kinds.contains(&Kind::Bookmarks) {
            let mut stmt = conn
                .prepare(
                    "SELECT b.id, b.parent, b.type, COALESCE(b.title, ''), p.url, b.guid
                     FROM moz_bookmarks b LEFT JOIN moz_places p ON p.id = b.fk
                     ORDER BY b.parent, b.position",
                )
                .map_err(sql)?;
            let rows: Vec<Row> = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .map_err(sql)?
                .collect::<Result<_, _>>()
                .map_err(sql)?;
            let root = |guid: &str| rows.iter().find(|r| r.5 == guid).map(|r| r.0);
            let tree = |id: i64| subtree(&rows, id);
            out.bar = root("toolbar_____").map(tree).unwrap_or_default();
            let mut other = Vec::new();
            for guid in ["menu________", "unfiled_____"] {
                other.extend(root(guid).map(tree).unwrap_or_default());
            }
            out.other = other;
        }
        if kinds.contains(&Kind::History) {
            let from = since.map_or(i64::MIN, |ms| ms * 1000);
            let mut stmt = conn
                .prepare(
                    "SELECT p.url, COALESCE(p.title, ''), v.visit_date FROM moz_historyvisits v
                     JOIN moz_places p ON p.id = v.place_id WHERE v.visit_date >= ?1 ORDER BY v.visit_date",
                )
                .map_err(sql)?;
            out.visits = stmt
                .query_map([from], |r| {
                    Ok(Visit {
                        url: r.get(0)?,
                        title: r.get(1)?,
                        at: crate::from_firefox(r.get(2)?),
                    })
                })
                .map_err(sql)?
                .collect::<Result<_, _>>()
                .map_err(sql)?;
        }
    }
    if kinds.contains(&Kind::SearchEngines) && profile.join("search.json.mozlz4").exists() {
        let (engines, default) = engines(&mozlz4(&profile.join("search.json.mozlz4"))?);
        out.engines = engines;
        out.default_engine = default;
    }
    let session = profile.join("sessionstore-backups/recovery.jsonlz4");
    if kinds.contains(&Kind::Tabs) && session.exists() {
        out.windows = windows(&mozlz4(&session)?);
    }
    if kinds.contains(&Kind::Extensions)
        && let Ok(text) = std::fs::read_to_string(profile.join("extensions.json"))
        && let Ok(v) = serde_json::from_str::<Value>(&text)
    {
        // Listed only: Firefox add-ons come from another store (12 §5.2).
        out.extensions = v["addons"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| a["location"] == "app-profile" && a["type"] == "extension")
            .filter_map(|a| {
                Some(crate::Extension {
                    id: a["id"].as_str()?.to_owned(),
                    name: a["defaultLocale"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                })
            })
            .collect();
    }
    Ok(out)
}

/// Bookmarks (type 1) and folders (type 2) under `parent`, in order.
fn subtree(rows: &[Row], parent: i64) -> Vec<Bookmark> {
    rows.iter()
        .filter(|r| r.1 == parent && r.0 != parent)
        .filter_map(|r| match (r.2, &r.4) {
            (1, Some(url)) if !url.starts_with("place:") => Some(Bookmark::link(&r.3, url)),
            (2, _) => Some(Bookmark::folder(&r.3, subtree(rows, r.0))),
            _ => None,
        })
        .collect()
}

/// `search.json`'s engines with a `text/html` URL, and the default's name.
pub fn engines(v: &Value) -> (Vec<Engine>, Option<String>) {
    let mut out = Vec::new();
    let mut default = None;
    let current_id = v["metaData"]["defaultEngineId"].as_str();
    let current_name = v["metaData"]["current"].as_str();
    for e in v["engines"].as_array().into_iter().flatten() {
        let name = e["_name"].as_str().unwrap_or_default().to_owned();
        if current_id.is_some_and(|id| e["id"].as_str() == Some(id))
            || current_name == Some(name.as_str())
        {
            default = Some(name.clone());
        }
        let Some(url) = e["_urls"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|u| u["type"].as_str().is_none_or(|t| t == "text/html"))
        else {
            continue;
        };
        let Some(template) = url["template"].as_str() else {
            continue;
        };
        let params: Vec<String> = url["params"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some(format!("{}={}", p["name"].as_str()?, p["value"].as_str()?)))
            .collect();
        let search_url = if params.is_empty() {
            template.to_owned()
        } else {
            let sep = if template.contains('?') { '&' } else { '?' };
            format!("{template}{sep}{}", params.join("&"))
        };
        if !search_url.contains("{searchTerms}") {
            continue;
        }
        out.push(Engine {
            name,
            keyword: e["_metaData"]["alias"]
                .as_str()
                .or_else(|| e["_definedAliases"][0].as_str())
                .unwrap_or_default()
                .to_owned(),
            search_url,
        });
    }
    (out, default)
}

/// The session's windows: each tab's current entry (`index` is 1-based).
pub fn windows(v: &Value) -> Vec<Vec<Tab>> {
    v["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| {
            w["tabs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| {
                    let entries = t["entries"].as_array()?;
                    let i = t["index"]
                        .as_u64()
                        .and_then(|i| usize::try_from(i).ok())
                        .unwrap_or(entries.len());
                    let e = entries
                        .get(i.saturating_sub(1))
                        .or_else(|| entries.last())?;
                    Some(Tab {
                        url: e["url"].as_str()?.to_owned(),
                        title: e["title"].as_str().unwrap_or_default().to_owned(),
                        pinned: t["pinned"].as_bool().unwrap_or(false),
                        group: t["groupId"].as_str().map(str::to_owned),
                    })
                })
                .collect::<Vec<_>>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}
