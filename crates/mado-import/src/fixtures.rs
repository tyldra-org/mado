//! The fixture homes in `tests/import/` turned into what each browser
//! writes (see `tests/import/README.md`). For tests only: behind the
//! `fixtures` feature.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// `tests/import/<browser>` copied into a temporary home, with SQL dumps,
/// session descriptions and JSON for mozLz4 turned into their real formats.
pub fn home(browser: &str) -> tempfile::TempDir {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/import")
        .join(browser);
    let to = tempfile::tempdir().expect("a temporary home");
    copy(&from, to.path());
    to
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("folder");
    for entry in std::fs::read_dir(from).expect("fixture folder").flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            copy(&path, &to.join(&name));
        } else if let Some(base) = name.strip_suffix(".sql") {
            let conn = rusqlite::Connection::open(to.join(base)).expect("database");
            conn.execute_batch(&std::fs::read_to_string(&path).expect("sql"))
                .expect("the dump runs");
        } else if let Some(base) = name.strip_suffix(".snss.json") {
            let v: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).expect("json")).expect("json");
            std::fs::write(to.join(base), snss(&v)).expect("write");
        } else if let Some(base) = name
            .strip_suffix(".json")
            .filter(|b| b.ends_with(".mozlz4") || b.ends_with(".jsonlz4"))
        {
            std::fs::write(to.join(base), mozlz4(&std::fs::read(&path).expect("json")))
                .expect("write");
        } else {
            let base = name.strip_suffix(".fixture").unwrap_or(&name);
            std::fs::copy(&path, to.join(base)).expect("copy");
        }
    }
}

/// A mozLz4 file of `json`.
pub fn mozlz4(json: &[u8]) -> Vec<u8> {
    let mut out = b"mozLz40\0".to_vec();
    out.extend(u32::try_from(json.len()).expect("small").to_le_bytes());
    out.extend(lz4_flex::block::compress(json));
    out
}

fn command(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.extend(
        u16::try_from(payload.len() + 1)
            .expect("small")
            .to_le_bytes(),
    );
    out.push(id);
    out.extend(payload);
}

fn pad(b: &mut Vec<u8>) {
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
}

fn i32s(values: &[i64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|v| i32::try_from(*v).expect("an ID").to_le_bytes())
        .collect()
}

/// An SNSS file from `{ windows: [{ id, closed?, tabs: [{ id, index,
/// pages: [[url, title]], selected?, pinned?, group?: [high, low],
/// closed? }] }] }`, written as Chromium writes its commands.
pub fn snss(v: &Value) -> Vec<u8> {
    let mut out = b"SNSS".to_vec();
    out.extend(3_i32.to_le_bytes());
    for w in v["windows"].as_array().into_iter().flatten() {
        let wid = w["id"].as_i64().unwrap_or(0);
        for t in w["tabs"].as_array().into_iter().flatten() {
            let tid = t["id"].as_i64().unwrap_or(0);
            command(&mut out, 0, &i32s(&[wid, tid]));
            command(&mut out, 2, &i32s(&[tid, t["index"].as_i64().unwrap_or(0)]));
            for (i, page) in t["pages"].as_array().into_iter().flatten().enumerate() {
                let url = page[0].as_str().unwrap_or_default();
                let title: Vec<u16> = page[1]
                    .as_str()
                    .unwrap_or_default()
                    .encode_utf16()
                    .collect();
                let mut p = i32s(&[tid, i64::try_from(i).expect("small")]);
                p.extend(i32::try_from(url.len()).expect("small").to_le_bytes());
                p.extend(url.as_bytes());
                pad(&mut p);
                p.extend(i32::try_from(title.len()).expect("small").to_le_bytes());
                p.extend(title.iter().flat_map(|u| u.to_le_bytes()));
                pad(&mut p);
                // Fields after the title, which the reader skips.
                p.extend(i32s(&[0, 0]));
                let mut pickle = u32::try_from(p.len())
                    .expect("small")
                    .to_le_bytes()
                    .to_vec();
                pickle.extend(p);
                command(&mut out, 6, &pickle);
            }
            if let Some(s) = t["selected"].as_i64() {
                command(&mut out, 7, &i32s(&[tid, s]));
            }
            if t["pinned"].as_bool() == Some(true) {
                let mut p = i32s(&[tid]);
                p.extend([1, 0, 0, 0]);
                command(&mut out, 12, &p);
            }
            if let Some(g) = t["group"].as_array() {
                let mut p = i32s(&[tid, 0]);
                for part in g {
                    p.extend(part.as_u64().unwrap_or(0).to_le_bytes());
                }
                p.extend([1, 0, 0, 0, 0, 0, 0, 0]);
                command(&mut out, 25, &p);
            }
            if t["closed"].as_bool() == Some(true) {
                let mut p = i32s(&[tid, 0]);
                p.extend(0_i64.to_le_bytes());
                command(&mut out, 16, &p);
            }
        }
        if w["closed"].as_bool() == Some(true) {
            let mut p = i32s(&[wid, 0]);
            p.extend(0_i64.to_le_bytes());
            command(&mut out, 17, &p);
        }
        // A command the reader does not know is skipped.
        command(&mut out, 21, &i32s(&[wid, 0, 0]));
    }
    out
}

/// The path of a file inside a fixture home.
pub fn path(home: &Path, rest: &str) -> PathBuf {
    home.join(rest)
}
