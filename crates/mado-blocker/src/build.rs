//! The shared engine build and its cache (10 §3).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use mado_lists::FilterList;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::lists::{filter_count, resources, stored};

/// A serialised engine and what it was built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    pub hash: String,
    pub bytes: Vec<u8>,
    pub filters: usize,
    /// The bundled EasyList stood in for lists never downloaded.
    pub bundled: bool,
    /// WebKit content rule lists, `(identifier, JSON file)` (10 §4).
    pub shards: Vec<(String, PathBuf)>,
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("the new build has {got} filters, under half of {had}; the previous build stays")]
    Regression { got: usize, had: usize },
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// The last build, so a later one can be compared with it.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Last {
    hash: String,
    filters: usize,
}

fn last(dir: &Path) -> Option<Last> {
    serde_json::from_slice(&std::fs::read(dir.join("build.json")).ok()?).ok()
}

fn engine_path(dir: &Path, hash: &str) -> PathBuf {
    dir.join(format!("engine-{hash}.dat"))
}

/// The list texts in use: the stored downloads, or the bundled EasyList
/// when none was ever downloaded.
fn texts(dir: &Path, lists: &[FilterList]) -> (Vec<(String, String)>, bool) {
    let found: Vec<(String, String)> = lists
        .iter()
        .filter_map(|l| stored(dir, &l.id).map(|(text, _)| (l.id.clone(), text)))
        .collect();
    if found.is_empty() {
        (
            vec![("easylist".to_owned(), crate::bundled_easylist())],
            true,
        )
    } else {
        (found, false)
    }
}

fn hash_of(texts: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = texts.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    // The serialised format belongs to this adblock version.
    h.update(b"adblock-0.13\0");
    for (id, text) in sorted {
        h.update(id.as_bytes());
        h.update([0]);
        h.update(Sha256::digest(text.as_bytes()));
    }
    h.finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Holds `<dir>/.lock` while it lives.
struct Lock(std::fs::File);

impl Lock {
    fn take(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(".lock"))?;
        file.lock()?;
        Ok(Self(file))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// The engine for `lists` stored in `dir` (10 §3): the cached file when
/// its hash matches, else a new build, written atomically under the lock.
/// A build with under half the filters of the last one is refused, and the
/// last one is returned with the error.
pub fn build(dir: &Path, lists: &[FilterList]) -> Result<Build, (BuildError, Option<Build>)> {
    let lock = Lock::take(dir).map_err(|e| (BuildError::Io(e), None))?;
    let (texts, bundled) = texts(dir, lists);
    let filters: usize = texts.iter().map(|(_, t)| filter_count(t)).sum();
    let hash = hash_of(&texts);
    let previous = last(dir);
    let previous_build = |p: &Last| {
        std::fs::read(engine_path(dir, &p.hash))
            .ok()
            .map(|bytes| Build {
                hash: p.hash.clone(),
                bytes,
                filters: p.filters,
                bundled: false,
                shards: shard_files(dir, &p.hash),
            })
    };
    if let Some(p) = &previous
        && p.hash != hash
        && filters * 2 < p.filters
    {
        tracing::warn!(got = filters, had = p.filters, "listRegression");
        return Err((
            BuildError::Regression {
                got: filters,
                had: p.filters,
            },
            previous_build(p),
        ));
    }
    let path = engine_path(dir, &hash);
    let mut shards = shard_files(dir, &hash);
    if shards.is_empty() {
        let (rules, skipped) = crate::rules::convert(texts.iter().map(|(_, t)| t.clone()));
        tracing::info!(rules = rules.len(), skipped, "filters converted for WebKit");
        for (n, json) in crate::rules::shard(rules, crate::rules::MAX_RULES)
            .iter()
            .enumerate()
        {
            let file = dir.join(format!("rules-{hash}-{n}.json"));
            write_file(&file, json.as_bytes()).map_err(|e| (BuildError::Io(e), None))?;
        }
        shards = shard_files(dir, &hash);
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) if engine_from(&bytes, Vec::new()).is_some() => bytes,
        _ => {
            let mut set = adblock::lists::FilterSet::new(false);
            for (_, text) in texts {
                set.add_filter_list(text, adblock::lists::ParseOptions::default());
            }
            let bytes = adblock::Engine::new_with_filter_set(set).serialize();
            write_file(&path, &bytes).map_err(|e| (BuildError::Io(e), None))?;
            bytes
        }
    };
    let record = serde_json::to_vec(&Last {
        hash: hash.clone(),
        filters,
    })
    .unwrap_or_default();
    let _ = std::fs::write(dir.join("build.json"), record);
    // Older engines go; another process may still be reading one, which
    // the open file survives.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let stale = (name.starts_with("engine-") && name.ends_with(".dat") && e.path() != path)
                || (name.starts_with("rules-") && !name.starts_with(&format!("rules-{hash}-")));
            if stale {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    drop(lock);
    Ok(Build {
        hash,
        bytes,
        filters,
        bundled,
        shards,
    })
}

fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// The rule list files of build `hash`, in order, with their WebKit
/// identifiers `mado-<hash>-<n>`.
fn shard_files(dir: &Path, hash: &str) -> Vec<(String, PathBuf)> {
    (0..)
        .map(|n| {
            (
                format!("mado-{hash}-{n}"),
                dir.join(format!("rules-{hash}-{n}.json")),
            )
        })
        .take_while(|(_, p)| p.exists())
        .collect()
}

/// An engine from serialised bytes, with the stored resources (scriptlets
/// and redirects). `None` for corrupt bytes.
pub fn engine_from(
    bytes: &[u8],
    resources: Vec<adblock::resources::Resource>,
) -> Option<adblock::Engine> {
    let mut engine = adblock::Engine::default();
    engine.deserialize(bytes).ok()?;
    engine.use_resources(resources);
    Some(engine)
}

/// The stored resources, for [`engine_from`].
pub fn stored_resources(dir: &Path) -> Vec<adblock::resources::Resource> {
    resources(dir)
}

/// A profile's own filters (10 §3): custom filters and hidden elements,
/// checked after the shared engine.
pub fn custom_engine(
    lines: &[String],
    resources: Vec<adblock::resources::Resource>,
) -> adblock::Engine {
    let mut set = adblock::lists::FilterSet::new(false);
    set.add_filter_list(lines.join("\n"), adblock::lists::ParseOptions::default());
    let mut engine = adblock::Engine::new_with_filter_set(set);
    engine.use_resources(resources);
    engine
}
