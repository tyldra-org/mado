//! The session in the profile database (spec 03 §4): windows, tabs, groups,
//! closed saved groups and the closed stack. The core writes what changed
//! after each handler, so a crash loses at most the writes in flight, and
//! loads it all once at startup.

use std::collections::BTreeMap;

use mado_engine::tab::SessionBlob;
use mado_types::state::{
    Bounds, DisplayUrl, GroupColor, GroupState, Sidebar, SidebarMode, TabLayout, TabLife, TabState,
    WindowKind, WindowState, WindowStateKind,
};
use mado_types::{GroupId, TabId, WindowId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::tabs::{ClosedEntry, ClosedGroup, Model, TabExtra, TabSnapshot};

/// What the last run left.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Loaded {
    pub model: Model,
    /// Oldest first; the newest is popped first.
    pub closed: Vec<(i64, ClosedEntry)>,
}

fn word<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn parse<T: DeserializeOwned + Default>(text: &str) -> T {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).unwrap_or_default()
}

fn id<T: std::str::FromStr>(text: Option<String>) -> Option<T> {
    text.and_then(|t| t.parse().ok())
}

#[allow(clippy::cast_possible_truncation)]
fn int(v: f64) -> i64 {
    v.round() as i64
}

#[allow(clippy::cast_precision_loss)]
fn float(v: Option<i64>) -> f64 {
    v.unwrap_or(0) as f64
}

/// Reads the saved session. Every tab comes back sleeping; the core wakes
/// the ones it shows.
pub fn load(conn: &Connection) -> rusqlite::Result<Loaded> {
    let mut model = Model::default();
    let mut order: BTreeMap<WindowId, Vec<(i64, TabId)>> = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT id, kind, state, x, y, width, height, screen, layout, sidebar_width, sidebar_mode, active_tab_id
         FROM windows ORDER BY sort",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            [r.get::<_, Option<i64>>(3)?, r.get(4)?, r.get(5)?, r.get(6)?],
            r.get::<_, Option<String>>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, Option<i64>>(9)?,
            r.get::<_, String>(10)?,
            r.get::<_, Option<String>>(11)?,
        ))
    })?;
    for row in rows {
        let (wid, kind, state, xywh, screen, layout, sidebar_width, sidebar_mode, active) = row?;
        let Some(wid) = id::<WindowId>(Some(wid)) else {
            continue;
        };
        model.windows.insert(
            wid,
            WindowState {
                id: wid,
                kind: parse::<WindowKind>(&kind),
                bounds: Bounds {
                    x: float(xywh[0]),
                    y: float(xywh[1]),
                    width: float(xywh[2]),
                    height: float(xywh[3]),
                    screen: screen.unwrap_or_default(),
                },
                state: parse::<WindowStateKind>(&state),
                layout: parse::<TabLayout>(&layout),
                sidebar: Sidebar {
                    width: sidebar_width.map_or(240.0, |w| float(Some(w))),
                    mode: parse::<SidebarMode>(&sidebar_mode),
                },
                active_tab_id: id(active),
                ..WindowState::default()
            },
        );
    }
    let mut stmt = conn.prepare(
        "SELECT id, window_id, group_id, pinned, sort, url, title, custom_title, favicon_url,
                pinned_home_url, opener_id, zoom, muted, session_blob, scroll_x, scroll_y,
                last_active_at, created_at
         FROM tabs",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            (
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ),
            (
                r.get::<_, String>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, f64>(11)?,
                r.get::<_, bool>(12)?,
            ),
            (
                r.get::<_, Option<Vec<u8>>>(13)?,
                r.get::<_, Option<i64>>(14)?,
                r.get::<_, Option<i64>>(15)?,
                r.get::<_, i64>(16)?,
                r.get::<_, i64>(17)?,
            ),
        ))
    })?;
    for row in rows {
        let (
            (tid, wid, gid, pinned, sort, url),
            (title, custom, favicon, home, opener, zoom, muted),
            (blob, sx, sy, last, created),
        ) = row?;
        let (Some(tid), Some(wid)) = (id::<TabId>(Some(tid)), id::<WindowId>(Some(wid))) else {
            continue;
        };
        if !model.windows.contains_key(&wid) {
            continue;
        }
        let mut tab = TabState {
            id: tid,
            window_id: wid,
            group_id: id(gid),
            pinned,
            opener_id: id(opener),
            created_at: created,
            last_active_at: last,
            title,
            custom_title: custom,
            display_url: DisplayUrl {
                text: url.clone(),
                ..DisplayUrl::default()
            },
            url,
            favicon,
            life: TabLife::Sleeping,
            zoom,
            ..TabState::default()
        };
        tab.audio.muted = muted;
        model.tabs.insert(tid, tab);
        model.extras.insert(
            tid,
            TabExtra {
                pinned_home_url: home,
                session: blob.map(SessionBlob),
                snapshot: None,
                scroll: sx.zip(sy),
            },
        );
        order.entry(wid).or_default().push((sort, tid));
    }
    for (wid, mut tabs) in order {
        tabs.sort_unstable();
        if let Some(w) = model.windows.get_mut(&wid) {
            w.tab_order = tabs.into_iter().map(|(_, t)| t).collect();
        }
    }
    let mut stmt = conn.prepare(
        "SELECT id, window_id, title, color, collapsed, saved, open FROM groups WHERE agent_task_id IS NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, bool>(4)?,
            r.get::<_, bool>(5)?,
            r.get::<_, bool>(6)?,
        ))
    })?;
    for row in rows {
        let (gid, wid, title, color, collapsed, saved, open) = row?;
        let Some(gid) = id::<GroupId>(Some(gid)) else {
            continue;
        };
        let group = GroupState {
            id: gid,
            window_id: id(wid),
            title,
            color: parse::<GroupColor>(&color),
            collapsed,
            saved,
            agent_task_id: None,
        };
        if open {
            model.groups.insert(gid, group);
        } else if saved {
            let mut tabs_stmt = conn.prepare(
                "SELECT url, title, favicon_url FROM saved_group_tabs WHERE group_id = ?1 ORDER BY sort",
            )?;
            let tabs = tabs_stmt
                .query_map([gid.to_string()], |r| {
                    Ok(TabSnapshot {
                        url: r.get(0)?,
                        title: r.get(1)?,
                        favicon: r.get(2)?,
                        pinned: false,
                        custom_title: None,
                        session_blob: None,
                        scroll: None,
                        pinned_home_url: None,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            model.closed_groups.insert(
                gid,
                ClosedGroup {
                    group: GroupState {
                        window_id: None,
                        ..group
                    },
                    tabs,
                },
            );
        }
    }
    // Drop rows that would break the model's invariants (a crash between
    // writes, an older build): tabs in missing groups, empty windows.
    for t in model.tabs.values_mut() {
        if t.pinned
            || t.group_id.is_some_and(|g| {
                model
                    .groups
                    .get(&g)
                    .is_none_or(|g| g.window_id != Some(t.window_id))
            })
        {
            t.group_id = None;
        }
    }
    let used: std::collections::BTreeSet<GroupId> =
        model.tabs.values().filter_map(|t| t.group_id).collect();
    model.groups.retain(|g, _| used.contains(g));
    model.windows.retain(|_, w| !w.tab_order.is_empty());
    for w in model.windows.values_mut() {
        if !w.active_tab_id.is_some_and(|a| w.tab_order.contains(&a)) {
            w.active_tab_id = w.tab_order.first().copied();
        }
    }
    let ids: Vec<WindowId> = model.windows.keys().copied().collect();
    for w in ids {
        model.close_gaps(w);
    }
    let mut stmt = conn.prepare("SELECT id, payload FROM closed_stack ORDER BY id")?;
    let closed = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| {
            let (i, payload) = r.ok()?;
            Some((i, serde_json::from_str(&payload).ok()?))
        })
        .collect();
    Ok(Loaded { model, closed })
}

/// Writes what changed between `before` and `after`.
pub fn write(conn: &Connection, before: &Model, after: &Model, now: i64) -> rusqlite::Result<()> {
    // Deletes first, children before parents.
    for t in before.tabs.keys().filter(|t| !after.tabs.contains_key(t)) {
        conn.execute("DELETE FROM tabs WHERE id = ?1", [t.to_string()])?;
    }
    for g in before.groups.keys().chain(before.closed_groups.keys()) {
        if !after.groups.contains_key(g) && !after.closed_groups.contains_key(g) {
            conn.execute("DELETE FROM groups WHERE id = ?1", [g.to_string()])?;
        }
    }
    for w in before
        .windows
        .keys()
        .filter(|w| !after.windows.contains_key(w))
    {
        conn.execute("DELETE FROM windows WHERE id = ?1", [w.to_string()])?;
    }
    for (sort, (wid, w)) in after.windows.iter().enumerate() {
        if before.windows.get(wid) == Some(w) {
            continue;
        }
        conn.execute(
            "INSERT INTO windows (id, kind, state, x, y, width, height, screen, layout, sidebar_width,
                 sidebar_mode, active_tab_id, sort, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)
             ON CONFLICT(id) DO UPDATE SET kind = ?2, state = ?3, x = ?4, y = ?5, width = ?6,
                 height = ?7, screen = ?8, layout = ?9, sidebar_width = ?10, sidebar_mode = ?11,
                 active_tab_id = ?12, sort = ?13, updated_at = ?14",
            params![
                wid.to_string(),
                word(&w.kind),
                word(&w.state),
                int(w.bounds.x),
                int(w.bounds.y),
                int(w.bounds.width),
                int(w.bounds.height),
                w.bounds.screen,
                word(&w.layout),
                int(w.sidebar.width),
                word(&w.sidebar.mode),
                w.active_tab_id.map(|t| t.to_string()),
                i64::try_from(sort).unwrap_or(0),
                now,
            ],
        )?;
    }
    for (gid, g) in &after.groups {
        if before.groups.get(gid) == Some(g) {
            continue;
        }
        write_group(conn, g, true, now)?;
        conn.execute(
            "DELETE FROM saved_group_tabs WHERE group_id = ?1",
            [gid.to_string()],
        )?;
    }
    for (gid, c) in &after.closed_groups {
        if before.closed_groups.get(gid) == Some(c) {
            continue;
        }
        write_group(conn, &c.group, false, now)?;
        conn.execute(
            "DELETE FROM saved_group_tabs WHERE group_id = ?1",
            [gid.to_string()],
        )?;
        for (i, t) in c.tabs.iter().enumerate() {
            conn.execute(
                "INSERT INTO saved_group_tabs (group_id, sort, url, title, favicon_url) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![gid.to_string(), i64::try_from(i).unwrap_or(0), t.url, t.title, t.favicon],
            )?;
        }
    }
    for (tid, t) in &after.tabs {
        let extra = after.extras.get(tid).cloned().unwrap_or_default();
        if before.tabs.get(tid) == Some(t) && before.extras.get(tid) == Some(&extra) {
            continue;
        }
        conn.execute(
            "INSERT INTO tabs (id, window_id, group_id, pinned, sort, url, title, custom_title, favicon_url,
                 pinned_home_url, opener_id, zoom, muted, session_blob, scroll_x, scroll_y, snapshot,
                 last_active_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
             ON CONFLICT(id) DO UPDATE SET window_id = ?2, group_id = ?3, pinned = ?4, url = ?5,
                 title = ?6, custom_title = ?7, favicon_url = ?8, pinned_home_url = ?9, opener_id = ?10,
                 zoom = ?11, muted = ?12, session_blob = ?13, scroll_x = ?14, scroll_y = ?15,
                 snapshot = ?16, last_active_at = ?17, updated_at = ?19",
            params![
                tid.to_string(),
                t.window_id.to_string(),
                t.group_id.map(|g| g.to_string()),
                t.pinned,
                t.url,
                t.title,
                t.custom_title,
                t.favicon,
                extra.pinned_home_url,
                t.opener_id.map(|o| o.to_string()),
                t.zoom,
                t.audio.muted,
                extra.session.map(|s| s.0),
                extra.scroll.map(|s| s.0),
                extra.scroll.map(|s| s.1),
                extra.snapshot,
                t.last_active_at,
                t.created_at,
                now,
            ],
        )?;
    }
    for (wid, w) in &after.windows {
        if before.windows.get(wid).map(|b| &b.tab_order) == Some(&w.tab_order)
            && w.tab_order.iter().all(|t| before.tabs.contains_key(t))
        {
            continue;
        }
        for (i, t) in w.tab_order.iter().enumerate() {
            conn.execute(
                "UPDATE tabs SET sort = ?2 WHERE id = ?1",
                params![t.to_string(), i64::try_from(i).unwrap_or(0)],
            )?;
        }
    }
    Ok(())
}

fn write_group(conn: &Connection, g: &GroupState, open: bool, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO groups (id, window_id, title, color, collapsed, saved, open, agent_task_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT(id) DO UPDATE SET window_id = ?2, title = ?3, color = ?4, collapsed = ?5,
             saved = ?6, open = ?7, agent_task_id = ?8, updated_at = ?9",
        params![
            g.id.to_string(),
            g.window_id.map(|w| w.to_string()),
            g.title,
            word(&g.color),
            g.collapsed,
            g.saved,
            open,
            g.agent_task_id.map(|t| t.to_string()),
            now,
        ],
    )?;
    Ok(())
}

/// Pushes an entry on the closed stack; keeps the newest 100.
pub fn push_closed(
    conn: &Connection,
    id: i64,
    entry: &ClosedEntry,
    now: i64,
) -> rusqlite::Result<()> {
    let payload = serde_json::to_string(entry)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT INTO closed_stack (id, kind, payload, closed_at) VALUES (?1, ?2, ?3, ?4)",
        params![id, entry.kind(), payload, now],
    )?;
    conn.execute(
        "DELETE FROM closed_stack WHERE id NOT IN (SELECT id FROM closed_stack ORDER BY id DESC LIMIT 100)",
        [],
    )?;
    Ok(())
}

pub fn pop_closed(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM closed_stack WHERE id = ?1", [id])?;
    Ok(())
}

/// `clean_quit.<profile>` in the app database.
pub fn clean_quit(conn: &Connection, profile: mado_types::ProfileId) -> rusqlite::Result<bool> {
    let v: Option<String> = conn
        .query_row(
            "SELECT v FROM app_state WHERE k = ?1",
            [format!("clean_quit.{profile}")],
            |r| r.get(0),
        )
        .optional()?;
    Ok(v.as_deref() != Some("0"))
}
