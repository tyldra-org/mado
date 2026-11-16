//! Filter list downloads (10 §2): conditional requests, a size cap, and a
//! guard that keeps the stored copy when a download has far fewer filters
//! than it, as a broken or truncated download would.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mado_lists::{FilterList, FilterLists};
use serde::{Deserialize, Serialize};

/// The largest list Mado downloads.
const MAX_LIST: usize = 20 * 1024 * 1024;
/// The largest resources file.
const MAX_RESOURCES: usize = 20 * 1024 * 1024;

/// The lists in use: `blocker.lists` when set, else the manifest's
/// defaults.
pub fn selected(manifest: &FilterLists, setting: &[String]) -> Vec<FilterList> {
    manifest
        .lists
        .iter()
        .filter(|l| {
            if setting.is_empty() {
                l.default
            } else {
                setting.contains(&l.id)
            }
        })
        .cloned()
        .collect()
}

/// Filters in a list: lines that are not empty, comments or headers.
pub fn filter_count(text: &str) -> usize {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('!') && !l.starts_with('['))
        .count()
}

/// What is kept with each list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stored {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub filters: usize,
    /// Unix milliseconds of the last download that changed it.
    pub updated_at: i64,
}

fn text_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("lists").join(format!("{id}.txt"))
}

fn meta_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("lists").join(format!("{id}.json"))
}

/// What is kept with `id`'s stored copy, if any.
pub fn stored_meta(dir: &Path, id: &str) -> Option<Stored> {
    stored(dir, id).map(|(_, m)| m)
}

pub(crate) fn stored(dir: &Path, id: &str) -> Option<(String, Stored)> {
    let text = std::fs::read_to_string(text_path(dir, id)).ok()?;
    let meta = std::fs::read(meta_path(dir, id))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    Some((text, meta))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// What an update did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    /// Lists whose text changed: a rebuild is due.
    pub changed: Vec<String>,
    /// Lists that could not be downloaded.
    pub failed: Vec<String>,
    /// Lists whose download was refused (`listRegression`).
    pub rejected: Vec<String>,
    pub resources_changed: bool,
}

enum Fetched {
    NotModified,
    Body {
        text: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

async fn fetch(
    client: &reqwest::Client,
    url: &str,
    meta: Option<&Stored>,
    max: usize,
) -> Result<Fetched, String> {
    let mut request = client.get(url);
    if let Some(m) = meta {
        if let Some(etag) = &m.etag {
            request = request.header("If-None-Match", etag);
        }
        if let Some(lm) = &m.last_modified {
            request = request.header("If-Modified-Since", lm);
        }
    }
    let mut response = request.send().await.map_err(|e| e.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Fetched::NotModified);
    }
    let response_ok = response.status().is_success();
    if !response_ok {
        return Err(format!("HTTP {}", response.status()));
    }
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let (etag, last_modified) = (header("etag"), header("last-modified"));
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > max {
            return Err(format!("over {max} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(body).map_err(|_| "not UTF-8".to_owned())?;
    Ok(Fetched::Body {
        text,
        etag,
        last_modified,
    })
}

/// Downloads `lists` and the manifest's `resources` into `dir` (10 §2
/// steps 1 and 2). A list that fails to download keeps its stored copy; so
/// does one with under half the filters of its stored copy, or none.
pub async fn update(
    client: &reqwest::Client,
    dir: &Path,
    lists: &[FilterList],
    resources: &[String],
    now: i64,
) -> UpdateReport {
    let mut report = UpdateReport::default();
    for list in lists {
        let old = stored(dir, &list.id);
        match fetch(client, &list.url, old.as_ref().map(|(_, m)| m), MAX_LIST).await {
            Ok(Fetched::NotModified) => {}
            Ok(Fetched::Body {
                text,
                etag,
                last_modified,
            }) => {
                let filters = filter_count(&text);
                let lower = text.trim_start().to_ascii_lowercase();
                let broken = filters == 0
                    || lower.starts_with("<!doctype")
                    || lower.starts_with("<html")
                    || old.as_ref().is_some_and(|(_, m)| filters * 2 < m.filters);
                if broken {
                    tracing::warn!(list = %list.id, filters, "listRegression");
                    report.rejected.push(list.id.clone());
                    continue;
                }
                if old.as_ref().is_some_and(|(t, _)| *t == text) {
                    continue;
                }
                let meta = Stored {
                    etag,
                    last_modified,
                    filters,
                    updated_at: now,
                };
                let saved =
                    write_atomic(&text_path(dir, &list.id), text.as_bytes()).and_then(|()| {
                        write_atomic(
                            &meta_path(dir, &list.id),
                            &serde_json::to_vec(&meta).unwrap_or_default(),
                        )
                    });
                match saved {
                    Ok(()) => report.changed.push(list.id.clone()),
                    Err(e) => {
                        tracing::warn!(list = %list.id, error = %e, "a list was not saved");
                        report.failed.push(list.id.clone());
                    }
                }
            }
            Err(e) => {
                tracing::warn!(list = %list.id, error = %e, "a list download failed");
                report.failed.push(list.id.clone());
            }
        }
    }
    let mut all = Vec::new();
    let mut ok = true;
    for url in resources {
        match fetch(client, url, None, MAX_RESOURCES).await {
            Ok(Fetched::Body { text, .. }) if !signature_ok(client, url, &text).await => {
                tracing::warn!(%url, "the blocker resources' signature does not verify");
                ok = false;
            }
            Ok(Fetched::Body { text, .. }) => {
                match serde_json::from_str::<Vec<adblock::resources::Resource>>(&text) {
                    Ok(r) => all.extend(r),
                    Err(e) => {
                        tracing::warn!(error = %e, "the blocker resources are malformed");
                        ok = false;
                    }
                }
            }
            Ok(Fetched::NotModified) => {}
            Err(e) => {
                tracing::warn!(error = %e, "the blocker resources did not download");
                ok = false;
            }
        }
    }
    if ok && !resources.is_empty() {
        let bytes = serde_json::to_vec(&all).unwrap_or_default();
        let path = dir.join("resources.json");
        if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            report.resources_changed = write_atomic(&path, &bytes).is_ok();
        }
    }
    report
}

/// Resources Mado publishes (`<lists URL>/<file>`) must carry a valid
/// `<file>.minisig` from the lists key: they are scripts pages run.
async fn signature_ok(client: &reqwest::Client, url: &str, text: &str) -> bool {
    let Some(file) = url.strip_prefix(mado_lists::URL) else {
        return true;
    };
    match fetch(client, &format!("{url}.minisig"), None, 4096).await {
        Ok(Fetched::Body { text: sig, .. }) => {
            mado_lists::verify_file(file, text.as_bytes(), &sig).is_ok()
        }
        _ => false,
    }
}

/// The stored resources.
pub(crate) fn resources(dir: &Path) -> Vec<adblock::resources::Resource> {
    std::fs::read(dir.join("resources.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// A client with the updater's timeout and no cookies.
pub fn client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
}
