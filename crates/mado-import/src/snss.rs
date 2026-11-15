//! Chromium's session files (`Sessions/Session_*`, "SNSS"): a header and
//! a stream of commands, each `u16` size, `u8` id and a payload. Only the
//! commands that place tabs, their pages, pins and groups are read; the
//! rest are skipped (spec 12 §5.2).

use std::collections::BTreeMap;

use crate::Tab;

const SET_TAB_WINDOW: u8 = 0;
const SET_TAB_INDEX_IN_WINDOW: u8 = 2;
const UPDATE_TAB_NAVIGATION: u8 = 6;
const SET_SELECTED_NAVIGATION_INDEX: u8 = 7;
const SET_PINNED_STATE: u8 = 12;
const TAB_CLOSED: u8 = 16;
const WINDOW_CLOSED: u8 = 17;
const SET_TAB_GROUP: u8 = 25;

#[derive(Default)]
struct TabState {
    window: Option<i32>,
    index: i32,
    pages: BTreeMap<i32, (String, String)>,
    selected: Option<i32>,
    pinned: bool,
    group: Option<String>,
}

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn pad4(n: usize) -> usize {
    n.div_ceil(4) * 4
}

/// `UpdateTabNavigation`'s pickle: `u32` payload size, then tab ID, index,
/// URL (UTF-8) and title (UTF-16), each length-prefixed and 4-aligned.
fn navigation(p: &[u8]) -> Option<(i32, i32, String, String)> {
    let tab = i32_at(p, 4)?;
    let index = i32_at(p, 8)?;
    let url_len = usize::try_from(i32_at(p, 12)?).ok()?;
    let url = std::str::from_utf8(p.get(16..16 + url_len)?)
        .ok()?
        .to_owned();
    let at = 16 + pad4(url_len);
    let title_len = usize::try_from(i32_at(p, at)?).ok()?;
    let units: Vec<u16> = p
        .get(at + 4..at + 4 + title_len * 2)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    Some((tab, index, url, String::from_utf16_lossy(&units)))
}

/// Windows of open tabs, in the order windows first appear.
pub fn parse(bytes: &[u8]) -> Result<Vec<Vec<Tab>>, String> {
    if bytes.get(..4) != Some(b"SNSS") {
        return Err("not an SNSS session file".into());
    }
    let mut tabs: BTreeMap<i32, TabState> = BTreeMap::new();
    let mut closed_windows = Vec::new();
    let mut at = 8;
    while at + 2 <= bytes.len() {
        let size = usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
        at += 2;
        let Some(command) = bytes.get(at..at + size) else {
            break;
        };
        at += size;
        let Some((&id, p)) = command.split_first() else {
            continue;
        };
        match id {
            SET_TAB_WINDOW => {
                if let (Some(w), Some(t)) = (i32_at(p, 0), i32_at(p, 4)) {
                    tabs.entry(t).or_default().window = Some(w);
                }
            }
            SET_TAB_INDEX_IN_WINDOW => {
                if let (Some(t), Some(i)) = (i32_at(p, 0), i32_at(p, 4)) {
                    tabs.entry(t).or_default().index = i;
                }
            }
            UPDATE_TAB_NAVIGATION => {
                if let Some((t, i, url, title)) = navigation(p) {
                    tabs.entry(t).or_default().pages.insert(i, (url, title));
                }
            }
            SET_SELECTED_NAVIGATION_INDEX => {
                if let (Some(t), Some(i)) = (i32_at(p, 0), i32_at(p, 4)) {
                    tabs.entry(t).or_default().selected = Some(i);
                }
            }
            SET_PINNED_STATE => {
                if let (Some(t), Some(&pinned)) = (i32_at(p, 0), p.get(4)) {
                    tabs.entry(t).or_default().pinned = pinned != 0;
                }
            }
            SET_TAB_GROUP => {
                if let (Some(t), Some(high), Some(low), Some(&has)) =
                    (i32_at(p, 0), u64_at(p, 8), u64_at(p, 16), p.get(24))
                {
                    tabs.entry(t).or_default().group =
                        (has != 0).then(|| format!("{high:016x}{low:016x}"));
                }
            }
            TAB_CLOSED => {
                if let Some(t) = i32_at(p, 0) {
                    tabs.remove(&t);
                }
            }
            WINDOW_CLOSED => {
                if let Some(w) = i32_at(p, 0) {
                    closed_windows.push(w);
                }
            }
            _ => {}
        }
    }
    let mut windows: Vec<(i32, Vec<(i32, Tab)>)> = Vec::new();
    for (_, t) in tabs {
        let Some(w) = t.window.filter(|w| !closed_windows.contains(w)) else {
            continue;
        };
        let page = t
            .selected
            .and_then(|s| t.pages.get(&s))
            .or_else(|| t.pages.values().next_back());
        let Some((url, title)) = page.cloned() else {
            continue;
        };
        let tab = Tab {
            url,
            title,
            pinned: t.pinned,
            group: t.group,
        };
        match windows.iter_mut().find(|(id, _)| *id == w) {
            Some((_, list)) => list.push((t.index, tab)),
            None => windows.push((w, vec![(t.index, tab)])),
        }
    }
    Ok(windows
        .into_iter()
        .map(|(_, mut list)| {
            list.sort_by_key(|(i, _)| *i);
            list.into_iter().map(|(_, t)| t).collect()
        })
        .collect())
}
