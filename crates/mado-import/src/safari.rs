//! Safari: `Bookmarks.plist` and `History.db` in `~/Library/Safari`,
//! which need Full Disk Access, and the default engine from Safari's
//! preferences when readable (spec 12 §5).

use std::path::Path;

use plist::Value;

use crate::{Bookmark, ImportError, Imported, Kind, Visit};

pub fn read(
    dir: &Path,
    kinds: &[Kind],
    since: Option<i64>,
    home: &Path,
) -> Result<Imported, ImportError> {
    let mut out = Imported::default();
    if kinds.contains(&Kind::Bookmarks) {
        let path = dir.join("Bookmarks.plist");
        let bytes = std::fs::read(&path).map_err(|e| ImportError::io(&path, e))?;
        let root = Value::from_reader(std::io::Cursor::new(bytes))
            .map_err(|e| ImportError::format(&path, e.to_string()))?;
        for list in children(&root) {
            match title(list).as_str() {
                "BookmarksBar" => out.bar = tree(list),
                "BookmarksMenu" => out.other.extend(tree(list)),
                // Reading List is not imported (12 §5.2).
                _ => {}
            }
        }
    }
    if kinds.contains(&Kind::History) {
        let path = dir.join("History.db");
        // Reading first tells a missing permission from a missing file.
        std::fs::File::open(&path).map_err(|e| ImportError::io(&path, e))?;
        let (conn, _tmp) = crate::sqlite::open_copy(&path)?;
        let sql = |e| ImportError::Sqlite(path.clone(), e);
        #[allow(clippy::cast_precision_loss)]
        let from = since.map_or(f64::MIN, |ms| ms as f64 / 1000.0 - 978_307_200.0);
        let mut stmt = conn
            .prepare(
                "SELECT i.url, COALESCE(v.title, ''), v.visit_time FROM history_visits v
                 JOIN history_items i ON i.id = v.history_item WHERE v.visit_time >= ?1 ORDER BY v.visit_time",
            )
            .map_err(sql)?;
        out.visits = stmt
            .query_map([from], |r| {
                Ok(Visit {
                    url: r.get(0)?,
                    title: r.get(1)?,
                    at: crate::from_safari(r.get(2)?),
                })
            })
            .map_err(sql)?
            .collect::<Result<_, _>>()
            .map_err(sql)?;
    }
    if kinds.contains(&Kind::SearchEngines) {
        let prefs = home.join(
            "Library/Containers/com.apple.Safari/Data/Library/Preferences/com.apple.Safari.plist",
        );
        out.default_engine = Value::from_file(&prefs).ok().and_then(|v| {
            v.as_dictionary()?
                .get("SearchProviderShortName")?
                .as_string()
                .map(str::to_owned)
        });
    }
    Ok(out)
}

fn children(v: &Value) -> Vec<&Value> {
    v.as_dictionary()
        .and_then(|d| d.get("Children"))
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn title(v: &Value) -> String {
    let d = v.as_dictionary();
    d.and_then(|d| d.get("Title"))
        .or_else(|| {
            d.and_then(|d| d.get("URIDictionary"))
                .and_then(Value::as_dictionary)
                .and_then(|u| u.get("title"))
        })
        .and_then(Value::as_string)
        .unwrap_or_default()
        .to_owned()
}

fn tree(list: &Value) -> Vec<Bookmark> {
    children(list)
        .into_iter()
        .filter_map(|n| {
            let d = n.as_dictionary()?;
            match d.get("WebBookmarkType")?.as_string()? {
                "WebBookmarkTypeLeaf" => {
                    Some(Bookmark::link(title(n), d.get("URLString")?.as_string()?))
                }
                "WebBookmarkTypeList" => Some(Bookmark::folder(title(n), tree(n))),
                _ => None,
            }
        })
        .collect()
}
