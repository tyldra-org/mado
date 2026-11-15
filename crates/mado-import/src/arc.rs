//! Arc's sidebar, `StorableSidebar.json` (spec 12 §5.2): favorites become
//! pinned tabs and each space's pinned tabs a saved group named after the
//! space. The file is not documented; this follows its shape as Arc writes
//! it: `sidebar.containers[]`, where one container holds `spaces` and
//! `items` as flat lists that alternate an ID and its object, and
//! `topAppsContainerIDs` names the favorites' container.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::{ImportError, Space, Tab};

/// Objects of a flat `[id, object, id, object, …]` list, by ID.
fn objects(list: &Value) -> HashMap<String, &Value> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| Some((v["id"].as_str()?.to_owned(), v)))
        .collect()
}

/// The IDs in a flat list that are strings, other than `skip`.
fn ids<'a>(list: &'a Value, skip: &[&str]) -> Vec<&'a str> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| !skip.contains(s))
        .collect()
}

/// Tabs under `container`, depth first, in sidebar order.
fn tabs(items: &HashMap<String, &Value>, container: &str, pinned: bool) -> Vec<Tab> {
    let Some(node) = items.get(container) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for child in node["childrenIds"].as_array().into_iter().flatten() {
        let Some(id) = child.as_str() else { continue };
        let Some(item) = items.get(id) else { continue };
        let tab = &item["data"]["tab"];
        if let Some(url) = tab["savedURL"].as_str() {
            out.push(Tab {
                url: url.to_owned(),
                title: item["title"]
                    .as_str()
                    .or_else(|| tab["savedTitle"].as_str())
                    .unwrap_or_default()
                    .to_owned(),
                pinned,
                group: None,
            });
        } else {
            out.extend(tabs(items, id, pinned));
        }
    }
    out
}

/// Favorites, and each space with its pinned tabs.
pub fn read(path: &Path) -> Result<(Vec<Tab>, Vec<Space>), ImportError> {
    let text = std::fs::read_to_string(path).map_err(|e| ImportError::io(path, e))?;
    let v: Value =
        serde_json::from_str(&text).map_err(|e| ImportError::format(path, e.to_string()))?;
    let Some(container) = v["sidebar"]["containers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| c.get("spaces").is_some() && c.get("items").is_some())
    else {
        return Err(ImportError::format(path, "no sidebar container"));
    };
    let items = objects(&container["items"]);
    let favorites = ids(&container["topAppsContainerIDs"], &["default"])
        .into_iter()
        .flat_map(|id| tabs(&items, id, true))
        .collect();
    let spaces = container["spaces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s.is_object())
        .filter_map(|s| {
            let ids: Vec<&str> = ids(&s["containerIDs"], &[]);
            let pinned = ids.windows(2).find(|w| w[0] == "pinned").map(|w| w[1])?;
            let list = tabs(&items, pinned, false);
            (!list.is_empty()).then(|| Space {
                title: s["title"].as_str().unwrap_or("Space").to_owned(),
                tabs: list,
            })
        })
        .collect();
    Ok((favorites, spaces))
}
