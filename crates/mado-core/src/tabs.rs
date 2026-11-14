//! The tab model (spec 04 §1-6): windows, their tab order, groups, the
//! opener rules for opening and closing, pinning, and what a closed tab,
//! group or window needs to come back.
//!
//! [`Model`] is the only owner of windows, tabs and groups. The core syncs
//! the state document from it after every handler ([`Model::sync`]), so
//! the UI sees exactly what the model holds. Every operation keeps the
//! invariants of 04 §1, which [`Model::check`] verifies in debug builds and
//! the property test verifies for random command sequences.

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use mado_engine::tab::SessionBlob;
use mado_types::command::{NamedPosition, TabPosition};
use mado_types::state::{
    Bounds, DisplayUrl, GroupColor, GroupState, SavedGroupSummary, Sidebar, StatePatch, TabLayout,
    TabLife, TabState, WindowKind, WindowState, WindowStateKind,
};
use mado_types::{ErrorCode, GroupId, MadoError, TabId, WindowId};
use serde::{Deserialize, Serialize};

/// Chrome's nine group colours, in the order `group.create` picks them.
pub const GROUP_COLORS: [GroupColor; 9] = [
    GroupColor::Grey,
    GroupColor::Blue,
    GroupColor::Red,
    GroupColor::Yellow,
    GroupColor::Green,
    GroupColor::Pink,
    GroupColor::Purple,
    GroupColor::Cyan,
    GroupColor::Orange,
];

/// What the core keeps about a tab beyond the state document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TabExtra {
    /// Where ⌘W on a pinned tab and `tab.resetPin` go back to.
    pub pinned_home_url: Option<String>,
    /// History and current page while sleeping, from the engine.
    pub session: Option<SessionBlob>,
    /// The hover-card picture taken before sleeping (JPEG).
    pub snapshot: Option<Vec<u8>>,
    pub scroll: Option<(i64, i64)>,
}

/// A tab as the closed stack and saved groups keep it (spec 04 §6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabSnapshot {
    pub url: String,
    pub title: String,
    pub favicon: Option<String>,
    pub pinned: bool,
    pub custom_title: Option<String>,
    /// Base64 of the engine's session blob.
    pub session_blob: Option<String>,
    pub scroll: Option<(i64, i64)>,
    /// Not in the spec's listing; restores pinned tabs' home page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_home_url: Option<String>,
}

impl TabSnapshot {
    fn of(tab: &TabState, extra: Option<&TabExtra>) -> Self {
        Self {
            url: tab.url.clone(),
            title: tab.title.clone(),
            favicon: tab.favicon.clone(),
            pinned: tab.pinned,
            custom_title: tab.custom_title.clone(),
            session_blob: extra
                .and_then(|e| e.session.as_ref())
                .map(|b| base64::engine::general_purpose::STANDARD.encode(&b.0)),
            scroll: extra.and_then(|e| e.scroll),
            pinned_home_url: extra.and_then(|e| e.pinned_home_url.clone()),
        }
    }

    fn extra(&self) -> TabExtra {
        TabExtra {
            pinned_home_url: self.pinned_home_url.clone(),
            session: self.session_blob.as_ref().and_then(|b| {
                base64::engine::general_purpose::STANDARD
                    .decode(b)
                    .ok()
                    .map(SessionBlob)
            }),
            snapshot: None,
            scroll: self.scroll,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSnapshot {
    pub id: GroupId,
    pub title: String,
    pub color: GroupColor,
    pub collapsed: bool,
    pub saved: bool,
}

impl GroupSnapshot {
    fn of(g: &GroupState) -> Self {
        Self {
            id: g.id,
            title: g.title.clone(),
            color: g.color,
            collapsed: g.collapsed,
            saved: g.saved,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSnapshot {
    pub bounds: Bounds,
    pub state: WindowStateKind,
    pub layout: TabLayout,
    pub sidebar_width: f64,
    /// Index in the window's tabs of the active tab.
    pub active: Option<usize>,
}

/// A tab inside a window or group entry, with its group (by index into the
/// entry's groups).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowTabSnapshot {
    #[serde(flatten)]
    pub tab: TabSnapshot,
    pub group: Option<GroupId>,
}

/// One entry of the closed stack (spec 04 §6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ClosedEntry {
    #[serde(rename_all = "camelCase")]
    Tab {
        window_id: WindowId,
        index: usize,
        group_id: Option<GroupId>,
        tab: TabSnapshot,
    },
    /// Several tabs closed at once (close others, close right). Spec 04 §4
    /// pushes them as one entry; the spec's listing has no kind for it, so
    /// it is `tabs` here (M2 roadmap, Spec issues).
    #[serde(rename_all = "camelCase")]
    Tabs {
        window_id: WindowId,
        tabs: Vec<(usize, Option<GroupId>, TabSnapshot)>,
    },
    #[serde(rename_all = "camelCase")]
    Group {
        window_id: WindowId,
        index: usize,
        group: GroupSnapshot,
        tabs: Vec<TabSnapshot>,
    },
    #[serde(rename_all = "camelCase")]
    Window {
        window: WindowSnapshot,
        tabs: Vec<WindowTabSnapshot>,
        groups: Vec<GroupSnapshot>,
    },
    #[serde(rename_all = "camelCase")]
    Session { windows: Vec<SessionWindow> },
}

impl ClosedEntry {
    /// The `closed_stack.kind` column; `tabs` is stored as `tab`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Tab { .. } | Self::Tabs { .. } => "tab",
            Self::Group { .. } => "group",
            Self::Window { .. } => "window",
            Self::Session { .. } => "session",
        }
    }

    /// A one-line summary for History › Recently closed.
    pub fn summary(&self) -> serde_json::Value {
        match self {
            Self::Tab { tab, .. } => {
                serde_json::json!({ "kind": "tab", "title": tab.title, "url": tab.url })
            }
            Self::Tabs { tabs, .. } => serde_json::json!({ "kind": "tabs", "count": tabs.len() }),
            Self::Group { group, tabs, .. } => {
                serde_json::json!({ "kind": "group", "title": group.title, "count": tabs.len() })
            }
            Self::Window { tabs, .. } => {
                serde_json::json!({ "kind": "window", "count": tabs.len() })
            }
            Self::Session { windows } => {
                serde_json::json!({ "kind": "session", "windows": windows.len() })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionWindow {
    pub window: WindowSnapshot,
    pub tabs: Vec<WindowTabSnapshot>,
    pub groups: Vec<GroupSnapshot>,
}

/// A saved group that is closed: not in any window.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedGroup {
    pub group: GroupState,
    pub tabs: Vec<TabSnapshot>,
}

/// A tab the model removed, for the closed stack and the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Removed {
    pub tab: TabState,
    pub extra: TabExtra,
    pub index: usize,
}

/// What a close did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Closed {
    pub tabs: Vec<Removed>,
    /// Windows left with no tabs, now closed.
    pub windows: Vec<WindowId>,
}

/// User preferences that steer the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Prefs {
    /// `tabs.newTabPosition` is `afterCurrent`.
    pub new_tab_after_current: bool,
    /// `tabs.ctrlTabOrder` is `recent`.
    pub ctrl_tab_recent: bool,
}

fn invalid(key: &str) -> MadoError {
    MadoError::new(ErrorCode::InvalidInput, key)
}

fn not_found(key: &str) -> MadoError {
    MadoError::new(ErrorCode::NotFound, key)
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub windows: BTreeMap<WindowId, WindowState>,
    pub tabs: BTreeMap<TabId, TabState>,
    pub groups: BTreeMap<GroupId, GroupState>,
    pub closed_groups: BTreeMap<GroupId, ClosedGroup>,
    pub extras: BTreeMap<TabId, TabExtra>,
    /// Per window: a tab opened from an opener, active since it opened with
    /// no other tab activated. Closing it returns to the opener (04 §4).
    return_to_opener: BTreeMap<WindowId, TabId>,
}

impl Model {
    // ----- queries -----

    pub fn window(&self, id: WindowId) -> Result<&WindowState, MadoError> {
        self.windows
            .get(&id)
            .ok_or_else(|| not_found("error.window.notFound"))
    }

    pub fn tab(&self, id: TabId) -> Result<&TabState, MadoError> {
        self.tabs
            .get(&id)
            .ok_or_else(|| not_found("error.tab.notFound"))
    }

    pub fn group(&self, id: GroupId) -> Result<&GroupState, MadoError> {
        self.groups
            .get(&id)
            .ok_or_else(|| not_found("error.group.notFound"))
    }

    fn index_of(&self, tab: TabId) -> Option<usize> {
        let w = self.tabs.get(&tab)?.window_id;
        self.windows
            .get(&w)?
            .tab_order
            .iter()
            .position(|t| *t == tab)
    }

    fn pinned_count(&self, window: WindowId) -> usize {
        self.windows.get(&window).map_or(0, |w| {
            w.tab_order
                .iter()
                .take_while(|t| self.tabs.get(t).is_some_and(|t| t.pinned))
                .count()
        })
    }

    fn group_of_at(&self, window: WindowId, index: usize) -> Option<GroupId> {
        let order = &self.windows.get(&window)?.tab_order;
        self.tabs.get(order.get(index)?)?.group_id
    }

    /// Tabs of `group` in strip order.
    pub fn group_tabs(&self, group: GroupId) -> Vec<TabId> {
        let Some(w) = self.groups.get(&group).and_then(|g| g.window_id) else {
            return Vec::new();
        };
        self.windows.get(&w).map_or_else(Vec::new, |w| {
            w.tab_order
                .iter()
                .copied()
                .filter(|t| self.tabs.get(t).is_some_and(|t| t.group_id == Some(group)))
                .collect()
        })
    }

    /// Tabs `tab.closeOthers` and `tab.closeRight` close: never pinned.
    pub fn others(&self, tab: TabId, right_only: bool) -> Result<Vec<TabId>, MadoError> {
        let t = self.tab(tab)?;
        let order = &self.window(t.window_id)?.tab_order;
        let at = order.iter().position(|x| *x == tab).unwrap_or(0);
        Ok(order
            .iter()
            .enumerate()
            .filter(|(i, x)| **x != tab && (!right_only || *i > at))
            .map(|(_, x)| *x)
            .filter(|x| self.tabs.get(x).is_some_and(|t| !t.pinned))
            .collect())
    }

    // ----- windows -----

    pub fn add_window(&mut self, id: WindowId, bounds: Bounds, layout: TabLayout, private: bool) {
        self.add_window_of(id, WindowKind::Normal, bounds, layout, private);
    }

    pub fn add_window_of(
        &mut self,
        id: WindowId,
        kind: WindowKind,
        bounds: Bounds,
        layout: TabLayout,
        private: bool,
    ) {
        self.windows.insert(
            id,
            WindowState {
                id,
                kind,
                bounds,
                layout,
                sidebar: Sidebar {
                    width: 240.0,
                    ..Sidebar::default()
                },
                is_private: private,
                ..WindowState::default()
            },
        );
    }

    /// Removes a window and everything in it; returns what it held, for the
    /// closed stack, and the tabs for the engine.
    pub fn remove_window(
        &mut self,
        id: WindowId,
    ) -> Result<(SessionWindow, Vec<TabId>), MadoError> {
        let snapshot = self.window_snapshot(id)?;
        let w = self
            .windows
            .remove(&id)
            .ok_or_else(|| not_found("error.window.notFound"))?;
        for t in &w.tab_order {
            self.tabs.remove(t);
            self.extras.remove(t);
        }
        self.groups.retain(|_, g| g.window_id != Some(id));
        self.return_to_opener.remove(&id);
        Ok((snapshot, w.tab_order))
    }

    pub fn window_snapshot(&self, id: WindowId) -> Result<SessionWindow, MadoError> {
        let w = self.window(id)?;
        let tabs: Vec<WindowTabSnapshot> = w
            .tab_order
            .iter()
            .filter_map(|t| {
                let tab = self.tabs.get(t)?;
                Some(WindowTabSnapshot {
                    tab: TabSnapshot::of(tab, self.extras.get(t)),
                    group: tab.group_id,
                })
            })
            .collect();
        let groups = self
            .groups
            .values()
            .filter(|g| g.window_id == Some(id))
            .map(GroupSnapshot::of)
            .collect();
        Ok(SessionWindow {
            window: WindowSnapshot {
                bounds: w.bounds.clone(),
                state: w.state,
                layout: w.layout,
                sidebar_width: w.sidebar.width,
                active: w
                    .active_tab_id
                    .and_then(|a| w.tab_order.iter().position(|t| *t == a)),
            },
            tabs,
            groups,
        })
    }

    // ----- opening -----

    /// Where a new tab goes (04 §3), and the group it joins.
    fn insertion(
        &self,
        window: WindowId,
        position: Option<TabPosition>,
        opener: Option<TabId>,
        prefs: Prefs,
    ) -> (usize, Option<GroupId>) {
        let Some(w) = self.windows.get(&window) else {
            return (0, None);
        };
        let len = w.tab_order.len();
        let pinned = self.pinned_count(window);
        let active_index = w
            .active_tab_id
            .and_then(|a| w.tab_order.iter().position(|t| *t == a));
        let opener_here =
            opener.filter(|o| self.tabs.get(o).is_some_and(|t| t.window_id == window));
        let (index, group) = match (position, opener_here) {
            (Some(TabPosition::Index { index }), _) => {
                (usize::try_from(index).unwrap_or(len).min(len), None)
            }
            (Some(TabPosition::Named(NamedPosition::AfterActive)), _) => {
                (active_index.map_or(len, |i| i + 1), None)
            }
            (Some(TabPosition::Named(NamedPosition::End)), _) => (len, None),
            (None, Some(o)) => {
                let mut i = w
                    .tab_order
                    .iter()
                    .position(|t| *t == o)
                    .map_or(len, |i| i + 1);
                while i < len && self.tabs.get(&w.tab_order[i]).and_then(|t| t.opener_id) == Some(o)
                {
                    i += 1;
                }
                let g = self
                    .tabs
                    .get(&o)
                    .filter(|t| !t.pinned)
                    .and_then(|t| t.group_id);
                (i, g)
            }
            (None, None) if prefs.new_tab_after_current => {
                (active_index.map_or(len, |i| i + 1), None)
            }
            (None, None) => (len, None),
        };
        let index = index.max(pinned);
        // A tab dropped between two tabs of a group joins it, so groups stay
        // contiguous.
        let inside = match (
            index
                .checked_sub(1)
                .and_then(|i| self.group_of_at(window, i)),
            self.group_of_at(window, index),
        ) {
            (Some(a), Some(b)) if a == b => Some(a),
            _ => None,
        };
        (index, inside.or(group))
    }

    /// Adds a tab; returns its ID. `foreground` makes it active.
    #[allow(clippy::too_many_arguments)]
    pub fn open_tab(
        &mut self,
        window: WindowId,
        url: String,
        position: Option<TabPosition>,
        foreground: bool,
        opener: Option<TabId>,
        now: i64,
        prefs: Prefs,
    ) -> Result<TabId, MadoError> {
        self.adopt_tab(
            TabId::new(),
            window,
            url,
            position,
            foreground,
            opener,
            now,
            prefs,
        )
    }

    /// Like `open_tab`, for a tab view the engine already made (a popup,
    /// spec 02 §4.3), whose ID it chose.
    #[allow(clippy::too_many_arguments)]
    pub fn adopt_tab(
        &mut self,
        id: TabId,
        window: WindowId,
        url: String,
        position: Option<TabPosition>,
        foreground: bool,
        opener: Option<TabId>,
        now: i64,
        prefs: Prefs,
    ) -> Result<TabId, MadoError> {
        self.window(window)?;
        if self.tabs.contains_key(&id) {
            return Err(invalid("error.tab.exists"));
        }
        let (index, group) = self.insertion(window, position, opener, prefs);
        let tab = TabState {
            id,
            window_id: window,
            group_id: group,
            opener_id: opener.filter(|o| self.tabs.contains_key(o)),
            created_at: now,
            last_active_at: now,
            display_url: DisplayUrl {
                text: url.clone(),
                ..DisplayUrl::default()
            },
            url,
            life: TabLife::Loading,
            zoom: 1.0,
            ..TabState::default()
        };
        self.tabs.insert(id, tab);
        self.extras.insert(id, TabExtra::default());
        if let Some(w) = self.windows.get_mut(&window) {
            w.tab_order.insert(index, id);
        }
        // An opener's group run may not reach the insertion point (its
        // children left the group): close the run back up.
        if group.is_some() {
            self.regroup(window);
        }
        let empty = self.window(window)?.active_tab_id.is_none();
        if foreground || empty {
            self.activate(id, now)?;
            if opener.is_some() && foreground {
                self.return_to_opener.insert(window, id);
            }
        }
        Ok(id)
    }

    /// Adds a tab restored from a snapshot, sleeping, at `index`. Returns
    /// its ID.
    pub fn restore_tab(
        &mut self,
        window: WindowId,
        index: usize,
        group: Option<GroupId>,
        snapshot: &TabSnapshot,
        now: i64,
    ) -> Result<TabId, MadoError> {
        self.window(window)?;
        let id = TabId::new();
        let pinned_count = self.pinned_count(window);
        let len = self.window(window)?.tab_order.len();
        let index = if snapshot.pinned {
            index.min(pinned_count)
        } else {
            index.clamp(pinned_count, len)
        };
        let group = group.filter(|g| {
            !snapshot.pinned
                && self
                    .groups
                    .get(g)
                    .is_some_and(|g| g.window_id == Some(window))
        });
        self.tabs.insert(
            id,
            TabState {
                id,
                window_id: window,
                group_id: None,
                pinned: snapshot.pinned,
                created_at: now,
                last_active_at: now,
                title: snapshot.title.clone(),
                custom_title: snapshot.custom_title.clone(),
                display_url: DisplayUrl {
                    text: snapshot.url.clone(),
                    ..DisplayUrl::default()
                },
                url: snapshot.url.clone(),
                favicon: snapshot.favicon.clone(),
                life: TabLife::Sleeping,
                zoom: 1.0,
                ..TabState::default()
            },
        );
        self.extras.insert(id, snapshot.extra());
        if let Some(w) = self.windows.get_mut(&window) {
            w.tab_order.insert(index, id);
        }
        match group {
            Some(g) => self.place_in_group(id, g),
            None => self.fix_group_of(id),
        }
        if self.window(window)?.active_tab_id.is_none() {
            self.activate(id, now)?;
        }
        Ok(id)
    }

    /// A tab inserted inside another group's run joins that group.
    fn fix_group_of(&mut self, tab: TabId) {
        let (Some(w), Some(i)) = (self.tabs.get(&tab).map(|t| t.window_id), self.index_of(tab))
        else {
            return;
        };
        let left = i.checked_sub(1).and_then(|j| self.group_of_at(w, j));
        let right = self.group_of_at(w, i + 1);
        let own = self.tabs.get(&tab).and_then(|t| t.group_id);
        let target = match (left, right) {
            (Some(a), Some(b)) if a == b => Some(a),
            _ if own.is_some() && (left == own || right == own) => own,
            _ => None,
        };
        if let Some(t) = self.tabs.get_mut(&tab) {
            t.group_id = if t.pinned { None } else { target };
        }
    }

    /// Moves `tab` to the end of `group`'s run and puts it in the group.
    fn place_in_group(&mut self, tab: TabId, group: GroupId) {
        let Some(w) = self.tabs.get(&tab).map(|t| t.window_id) else {
            return;
        };
        let members = self.group_tabs(group);
        let Some(win) = self.windows.get_mut(&w) else {
            return;
        };
        win.tab_order.retain(|t| *t != tab);
        let at = members
            .iter()
            .filter(|m| **m != tab)
            .filter_map(|m| win.tab_order.iter().position(|t| t == m))
            .max()
            .map_or(win.tab_order.len(), |i| i + 1);
        win.tab_order.insert(at, tab);
        if let Some(t) = self.tabs.get_mut(&tab) {
            t.group_id = Some(group);
        }
    }

    // ----- activation and selection -----

    /// Makes `tab` its window's active tab. Returns the tab it replaced.
    pub fn activate(&mut self, tab: TabId, now: i64) -> Result<Option<TabId>, MadoError> {
        let w = self.tab(tab)?.window_id;
        let win = self
            .windows
            .get_mut(&w)
            .ok_or_else(|| not_found("error.window.notFound"))?;
        let previous = win.active_tab_id.replace(tab);
        if let Some(t) = self.tabs.get_mut(&tab) {
            t.last_active_at = now;
            // The active tab never sleeps (04 §1): activating wakes it.
            if t.life == TabLife::Sleeping {
                t.life = TabLife::Loading;
            }
        }
        if self.return_to_opener.get(&w) != Some(&tab) {
            self.return_to_opener.remove(&w);
        }
        // A collapsed group shows its active tab, so it stays reachable.
        Ok(previous.filter(|p| *p != tab))
    }

    /// The tab after (or before) the active one, for Ctrl+Tab.
    pub fn neighbour(
        &self,
        window: WindowId,
        forward: bool,
        prefs: Prefs,
    ) -> Result<Option<TabId>, MadoError> {
        let w = self.window(window)?;
        let Some(active) = w.active_tab_id else {
            return Ok(None);
        };
        let visible: Vec<TabId> = w
            .tab_order
            .iter()
            .copied()
            .filter(|t| {
                *t == active
                    || !self
                        .tabs
                        .get(t)
                        .and_then(|t| t.group_id)
                        .and_then(|g| self.groups.get(&g))
                        .is_some_and(|g| g.collapsed)
            })
            .collect();
        let order: Vec<TabId> = if prefs.ctrl_tab_recent {
            let mut v = visible;
            v.sort_by_key(|t| std::cmp::Reverse(self.tabs.get(t).map_or(0, |t| t.last_active_at)));
            v
        } else {
            visible
        };
        let n = order.len();
        let Some(i) = order.iter().position(|t| *t == active) else {
            return Ok(None);
        };
        if n < 2 {
            return Ok(None);
        }
        Ok(Some(
            order[if forward {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            }],
        ))
    }

    /// ⌘1-8 picks a 0-based index; ⌘9 the last tab.
    pub fn at_index(
        &self,
        window: WindowId,
        index: usize,
        last: bool,
    ) -> Result<Option<TabId>, MadoError> {
        let order = &self.window(window)?.tab_order;
        Ok(if last {
            order.last().copied()
        } else {
            order.get(index).copied()
        })
    }

    /// Who becomes active when `closing` (the active tab) goes away with the
    /// other tabs in `gone` (04 §4).
    fn successor(&self, closing: TabId, gone: &BTreeSet<TabId>) -> Option<TabId> {
        let tab = self.tabs.get(&closing)?;
        let w = tab.window_id;
        let order = &self.windows.get(&w)?.tab_order;
        let at = order.iter().position(|t| *t == closing)?;
        let alive = |t: &TabId| !gone.contains(t);
        // 1. Back to the opener.
        if self.return_to_opener.get(&w) == Some(&closing)
            && let Some(o) = tab.opener_id
            && alive(&o)
            && self.tabs.get(&o).is_some_and(|t| t.window_id == w)
        {
            return Some(o);
        }
        // 2. Right, staying in the group while it has tabs left.
        if let Some(g) = tab.group_id {
            let in_group =
                |t: &&TabId| alive(t) && self.tabs.get(t).is_some_and(|x| x.group_id == Some(g));
            if let Some(t) = order[at + 1..].iter().find(in_group) {
                return Some(*t);
            }
            if let Some(t) = order[..at].iter().rev().find(in_group) {
                return Some(*t);
            }
        }
        // 2, 3. Right, then left.
        order[at + 1..]
            .iter()
            .find(|t| alive(t))
            .or_else(|| order[..at].iter().rev().find(|t| alive(t)))
            .copied()
    }

    // ----- closing -----

    /// ⌘W on a pinned tab with `tabs.closePinned` = `unload` (04 §4): the
    /// tab sleeps on its home page and another tab is selected. Returns
    /// `false` when no other tab can take over, so the caller closes it.
    pub fn unload_pinned(&mut self, tab: TabId, now: i64) -> Result<bool, MadoError> {
        let t = self.tab(tab)?;
        let w = t.window_id;
        if self.window(w)?.active_tab_id == Some(tab) {
            let Some(next) = self.successor(tab, &BTreeSet::from([tab])) else {
                return Ok(false);
            };
            self.activate(next, now)?;
        }
        let home = self
            .extras
            .get(&tab)
            .and_then(|e| e.pinned_home_url.clone());
        if let Some(t) = self.tabs.get_mut(&tab) {
            if let Some(home) = home {
                t.display_url.text.clone_from(&home);
                t.url = home;
            }
            t.life = TabLife::Sleeping;
        }
        let extra = self.extras.entry(tab).or_default();
        extra.session = None;
        extra.snapshot = None;
        Ok(true)
    }

    /// Closes `tabs`. Windows left empty close too (04 §4 rule 4).
    pub fn close(&mut self, tabs: &[TabId], now: i64) -> Closed {
        let gone: BTreeSet<TabId> = tabs
            .iter()
            .copied()
            .filter(|t| self.tabs.contains_key(t))
            .collect();
        let mut closed = Closed::default();
        // Choose successors first, while the order still holds every tab.
        let mut successors = Vec::new();
        for w in self.windows.values() {
            if let Some(a) = w.active_tab_id
                && gone.contains(&a)
            {
                let next = self.successor(a, &gone);
                // Returning to an opener keeps the chain: closing that tab
                // in turn returns to its own opener (Chrome).
                let back = next.is_some()
                    && self.return_to_opener.get(&w.id) == Some(&a)
                    && self.tabs.get(&a).and_then(|t| t.opener_id) == next;
                successors.push((w.id, next, back));
            }
        }
        for t in &gone {
            let index = self.index_of(*t).unwrap_or(0);
            if let Some(tab) = self.tabs.remove(t) {
                if let Some(w) = self.windows.get_mut(&tab.window_id) {
                    w.tab_order.retain(|x| x != t);
                }
                closed.tabs.push(Removed {
                    extra: self.extras.remove(t).unwrap_or_default(),
                    tab,
                    index,
                });
            }
        }
        closed.tabs.sort_by_key(|r| r.index);
        for (w, next, back) in successors {
            if let Some(win) = self.windows.get_mut(&w) {
                win.active_tab_id = None;
            }
            match next {
                Some(n) => {
                    let _ = self.activate(n, now);
                    if back && self.tabs.get(&n).is_some_and(|t| t.opener_id.is_some()) {
                        self.return_to_opener.insert(w, n);
                    }
                }
                None => {
                    self.windows.remove(&w);
                    self.groups.retain(|_, g| g.window_id != Some(w));
                    self.return_to_opener.remove(&w);
                    closed.windows.push(w);
                }
            }
        }
        // Openers that closed are forgotten; groups left empty go away.
        for t in self.tabs.values_mut() {
            if t.opener_id.is_some_and(|o| gone.contains(&o)) {
                t.opener_id = None;
            }
        }
        self.drop_empty_groups();
        closed
    }

    fn drop_empty_groups(&mut self) {
        let used: BTreeSet<GroupId> = self.tabs.values().filter_map(|t| t.group_id).collect();
        self.groups.retain(|id, _| used.contains(id));
    }

    // ----- moving -----

    /// `tab.move` (04 §2): keeps the tabs' relative order. `to_group`:
    /// `None` applies the drop rules, `Some(None)` leaves any group,
    /// `Some(Some(g))` joins `g`.
    pub fn move_tabs(
        &mut self,
        tabs: &[TabId],
        to_window: Option<WindowId>,
        to_index: usize,
        to_group: Option<Option<GroupId>>,
        now: i64,
    ) -> Result<Vec<WindowId>, MadoError> {
        if tabs.is_empty() {
            return Err(invalid("error.tab.noneGiven"));
        }
        for t in tabs {
            self.tab(*t)?;
        }
        let to_window = match to_window {
            Some(w) => {
                self.window(w)?;
                w
            }
            None => self.tab(tabs[0])?.window_id,
        };
        if let Some(Some(g)) = to_group
            && self.group(g)?.window_id != Some(to_window)
        {
            return Err(invalid("error.group.otherWindow"));
        }
        // In strip order, window by window.
        let mut moving: Vec<TabId> = Vec::new();
        for w in self.windows.values() {
            for t in &w.tab_order {
                if tabs.contains(t) && !moving.contains(t) {
                    moving.push(*t);
                }
            }
        }
        let set: BTreeSet<TabId> = moving.iter().copied().collect();
        // Sources keep a selection; a window that empties closes.
        let mut emptied = Vec::new();
        let mut new_active = Vec::new();
        for w in self.windows.values() {
            if w.id == to_window {
                continue;
            }
            if let Some(a) = w.active_tab_id
                && set.contains(&a)
            {
                new_active.push((w.id, self.successor(a, &set)));
            }
        }
        let target_index_tab = self.windows.get(&to_window).and_then(|w| {
            w.tab_order
                .iter()
                .filter(|t| !set.contains(t))
                .nth(to_index)
                .copied()
        });
        for w in self.windows.values_mut() {
            w.tab_order.retain(|t| !set.contains(t));
        }
        let old_groups: BTreeMap<TabId, Option<GroupId>> = moving
            .iter()
            .map(|t| (*t, self.tabs.get(t).and_then(|t| t.group_id)))
            .collect();
        for t in &moving {
            if let Some(tab) = self.tabs.get_mut(t) {
                tab.window_id = to_window;
                tab.group_id = None;
            }
        }
        // Pinned tabs go in the pinned block, the rest after it.
        let pinned_moving: Vec<TabId> = moving
            .iter()
            .copied()
            .filter(|t| self.tabs.get(t).is_some_and(|t| t.pinned))
            .collect();
        let rest: Vec<TabId> = moving
            .iter()
            .copied()
            .filter(|t| !pinned_moving.contains(t))
            .collect();
        let base = target_index_tab
            .and_then(|t| {
                self.windows
                    .get(&to_window)?
                    .tab_order
                    .iter()
                    .position(|x| *x == t)
            })
            .unwrap_or_else(|| {
                self.windows
                    .get(&to_window)
                    .map_or(0, |w| w.tab_order.len())
            });
        let pinned_count = self.pinned_count(to_window);
        for (at, t) in (base.min(pinned_count)..).zip(&pinned_moving) {
            if let Some(w) = self.windows.get_mut(&to_window) {
                w.tab_order.insert(at, *t);
            }
        }
        let pinned_count = self.pinned_count(to_window);
        let mut at = (base + pinned_moving.len()).max(pinned_count);
        // An explicit group places the tabs inside its run.
        if let Some(Some(g)) = to_group {
            let members = self.group_tabs(g);
            let positions: Vec<usize> = members
                .iter()
                .filter_map(|m| {
                    self.windows
                        .get(&to_window)?
                        .tab_order
                        .iter()
                        .position(|t| t == m)
                })
                .collect();
            if let (Some(lo), Some(hi)) = (positions.iter().min(), positions.iter().max()) {
                at = at.clamp(*lo, hi + 1);
            }
        }
        for t in &rest {
            if let Some(w) = self.windows.get_mut(&to_window) {
                let len = w.tab_order.len();
                w.tab_order.insert(at.min(len), *t);
            }
            at += 1;
        }
        for t in &rest {
            match to_group {
                Some(Some(g)) => {
                    if let Some(tab) = self.tabs.get_mut(t) {
                        tab.group_id = Some(g);
                    }
                }
                Some(None) => {}
                None => {
                    if let Some(tab) = self.tabs.get_mut(t) {
                        tab.group_id = old_groups.get(t).copied().flatten();
                    }
                    self.fix_group_of(*t);
                }
            }
        }
        // Moved tabs that stay in a group must sit inside its run: a group
        // that was split is closed back up around its first tab.
        self.regroup(to_window);
        for (w, next) in new_active {
            if let Some(win) = self.windows.get_mut(&w) {
                win.active_tab_id = None;
            }
            match next {
                Some(n) => {
                    let _ = self.activate(n, now);
                }
                None => emptied.push(w),
            }
        }
        for w in &emptied {
            self.windows.remove(w);
            self.return_to_opener.remove(w);
        }
        // Groups follow their tabs.
        let windows_of: BTreeMap<GroupId, WindowId> = self
            .tabs
            .values()
            .filter_map(|t| Some((t.group_id?, t.window_id)))
            .collect();
        for (g, w) in windows_of {
            if let Some(group) = self.groups.get_mut(&g) {
                group.window_id = Some(w);
            }
        }
        if self.window(to_window)?.active_tab_id.is_none()
            && let Some(first) = moving.first()
        {
            self.activate(*first, now)?;
        }
        self.drop_empty_groups();
        Ok(emptied)
    }

    /// Repairs a window read from disk: pinned tabs first, groups
    /// contiguous.
    pub fn close_gaps(&mut self, window: WindowId) {
        if let Some(w) = self.windows.get_mut(&window) {
            let tabs = &self.tabs;
            w.tab_order
                .sort_by_key(|t| !tabs.get(t).is_some_and(|t| t.pinned));
        }
        self.regroup(window);
    }

    /// Makes every group's tabs contiguous, keeping each run where its first
    /// tab is and the tabs' relative order.
    fn regroup(&mut self, window: WindowId) {
        let Some(w) = self.windows.get(&window) else {
            return;
        };
        let mut out: Vec<TabId> = Vec::with_capacity(w.tab_order.len());
        let mut placed: BTreeSet<GroupId> = BTreeSet::new();
        for t in &w.tab_order {
            let g = self.tabs.get(t).and_then(|x| x.group_id);
            match g {
                None => out.push(*t),
                Some(g) if placed.insert(g) => {
                    out.extend(
                        w.tab_order
                            .iter()
                            .filter(|x| self.tabs.get(x).and_then(|x| x.group_id) == Some(g)),
                    );
                }
                Some(_) => {}
            }
        }
        if let Some(w) = self.windows.get_mut(&window) {
            w.tab_order = out;
        }
    }

    // ----- pinning, renaming -----

    pub fn pin(&mut self, tabs: &[TabId], pinned: bool) -> Result<(), MadoError> {
        for t in tabs {
            self.tab(*t)?;
        }
        let mut ordered: Vec<TabId> = tabs.to_vec();
        ordered.sort_by_key(|t| self.index_of(*t));
        for t in ordered {
            let Some(tab) = self.tabs.get(&t) else {
                continue;
            };
            if tab.pinned == pinned {
                continue;
            }
            let w = tab.window_id;
            let url = tab.url.clone();
            if let Some(win) = self.windows.get_mut(&w) {
                win.tab_order.retain(|x| *x != t);
            }
            let at = self.pinned_count(w);
            if let Some(win) = self.windows.get_mut(&w) {
                win.tab_order.insert(at, t);
            }
            if let Some(tab) = self.tabs.get_mut(&t) {
                tab.pinned = pinned;
                tab.group_id = None;
            }
            let extra = self.extras.entry(t).or_default();
            extra.pinned_home_url = pinned.then_some(url);
        }
        self.drop_empty_groups();
        Ok(())
    }

    pub fn rename(&mut self, tab: TabId, title: &str) -> Result<(), MadoError> {
        let t = self
            .tabs
            .get_mut(&tab)
            .ok_or_else(|| not_found("error.tab.notFound"))?;
        let title = title.trim();
        if title.chars().count() > 200 {
            return Err(invalid("error.tab.titleTooLong"));
        }
        t.custom_title = (!title.is_empty()).then(|| title.to_owned());
        Ok(())
    }

    // ----- groups -----

    /// `group.create` (04 §5): the tabs move together to where the first
    /// of them is. Pinned tabs are unpinned first, as Chrome does.
    pub fn create_group(
        &mut self,
        tabs: &[TabId],
        title: Option<String>,
        color: Option<GroupColor>,
        now: i64,
    ) -> Result<GroupId, MadoError> {
        if tabs.is_empty() {
            return Err(invalid("error.tab.noneGiven"));
        }
        let first = *tabs
            .iter()
            .min_by_key(|t| (self.tabs.get(t).map(|t| t.window_id), self.index_of(**t)))
            .ok_or_else(|| invalid("error.tab.noneGiven"))?;
        let window = self.tab(first)?.window_id;
        self.pin(tabs, false)?;
        let id = GroupId::new();
        let used: Vec<GroupColor> = self
            .groups
            .values()
            .filter(|g| g.window_id == Some(window))
            .map(|g| g.color)
            .collect();
        let color = color.unwrap_or_else(|| {
            GROUP_COLORS
                .iter()
                .copied()
                .find(|c| !used.contains(c))
                .unwrap_or(GROUP_COLORS[used.len() % GROUP_COLORS.len()])
        });
        let at = self.index_of(first).unwrap_or(0);
        let at = self.windows.get(&window).map_or(0, |w| {
            w.tab_order[..at]
                .iter()
                .filter(|t| !tabs.contains(t))
                .count()
        });
        self.move_tabs(tabs, Some(window), at, Some(None), now)?;
        self.groups.insert(
            id,
            GroupState {
                id,
                window_id: Some(window),
                title: title.unwrap_or_default(),
                color,
                collapsed: false,
                saved: true,
                agent_task_id: None,
            },
        );
        for t in tabs {
            if let Some(tab) = self.tabs.get_mut(t) {
                tab.group_id = Some(id);
            }
        }
        self.regroup(window);
        Ok(id)
    }

    pub fn update_group(
        &mut self,
        id: GroupId,
        title: Option<String>,
        color: Option<GroupColor>,
        collapsed: Option<bool>,
    ) -> Result<(), MadoError> {
        let g = self
            .groups
            .get_mut(&id)
            .ok_or_else(|| not_found("error.group.notFound"))?;
        if g.agent_task_id.is_some() && title.is_some() {
            return Err(MadoError::new(
                ErrorCode::PermissionDenied,
                "error.group.agentRunning",
            ));
        }
        if let Some(t) = title {
            g.title = t;
        }
        if let Some(c) = color {
            g.color = c;
        }
        if let Some(c) = collapsed {
            g.collapsed = c;
        }
        Ok(())
    }

    pub fn ungroup(&mut self, id: GroupId) -> Result<(), MadoError> {
        self.group(id)?;
        for t in self.tabs.values_mut() {
            if t.group_id == Some(id) {
                t.group_id = None;
            }
        }
        self.groups.remove(&id);
        Ok(())
    }

    /// Closes a group's tabs. A saved group keeps its tabs as snapshots;
    /// returns what closed and, for an unsaved group, the entry for the
    /// closed stack.
    pub fn close_group(
        &mut self,
        id: GroupId,
        now: i64,
    ) -> Result<(Closed, Option<ClosedEntry>), MadoError> {
        let group = self.group(id)?.clone();
        let window = group
            .window_id
            .ok_or_else(|| not_found("error.group.notFound"))?;
        let members = self.group_tabs(id);
        let index = members.first().and_then(|t| self.index_of(*t)).unwrap_or(0);
        let snapshots: Vec<TabSnapshot> = members
            .iter()
            .filter_map(|t| Some(TabSnapshot::of(self.tabs.get(t)?, self.extras.get(t))))
            .collect();
        let closed = self.close(&members, now);
        if group.saved {
            self.closed_groups.insert(
                id,
                ClosedGroup {
                    group: GroupState {
                        window_id: None,
                        ..group
                    },
                    tabs: snapshots,
                },
            );
            Ok((closed, None))
        } else {
            let entry = ClosedEntry::Group {
                window_id: window,
                index,
                group: GroupSnapshot::of(&group),
                tabs: snapshots,
            };
            Ok((closed, Some(entry)))
        }
    }

    /// Reopens a closed saved group in `window`: its tabs sleep except the
    /// first, which becomes active.
    pub fn reopen_group(
        &mut self,
        id: GroupId,
        window: WindowId,
        now: i64,
    ) -> Result<Vec<TabId>, MadoError> {
        self.window(window)?;
        let closed = self
            .closed_groups
            .remove(&id)
            .ok_or_else(|| not_found("error.group.notFound"))?;
        self.groups.insert(
            id,
            GroupState {
                window_id: Some(window),
                ..closed.group
            },
        );
        let mut ids = Vec::new();
        for snap in &closed.tabs {
            let at = self.window(window)?.tab_order.len();
            let snap = TabSnapshot {
                pinned: false,
                ..snap.clone()
            };
            let t = self.restore_tab(window, at, Some(id), &snap, now)?;
            ids.push(t);
        }
        if ids.is_empty() {
            self.groups.remove(&id);
        } else if let Some(first) = ids.first() {
            self.activate(*first, now)?;
        }
        Ok(ids)
    }

    /// Restores a group entry from the closed stack.
    pub fn restore_group(
        &mut self,
        window: WindowId,
        index: usize,
        group: &GroupSnapshot,
        tabs: &[TabSnapshot],
        now: i64,
    ) -> Result<Vec<TabId>, MadoError> {
        let id = if self.groups.contains_key(&group.id) {
            GroupId::new()
        } else {
            group.id
        };
        self.groups.insert(
            id,
            GroupState {
                id,
                window_id: Some(window),
                title: group.title.clone(),
                color: group.color,
                collapsed: group.collapsed,
                saved: group.saved,
                agent_task_id: None,
            },
        );
        let mut ids = Vec::new();
        for (i, snap) in tabs.iter().enumerate() {
            let snap = TabSnapshot {
                pinned: false,
                ..snap.clone()
            };
            ids.push(self.restore_tab(window, index + i, Some(id), &snap, now)?);
        }
        if ids.is_empty() {
            self.groups.remove(&id);
        }
        self.regroup(window);
        Ok(ids)
    }

    /// Deletes a group for good; returns what it held so the core can
    /// offer Undo.
    pub fn delete_group(
        &mut self,
        id: GroupId,
        now: i64,
    ) -> Result<(Closed, ClosedEntry), MadoError> {
        if let Some(closed) = self.closed_groups.remove(&id) {
            return Ok((
                Closed::default(),
                ClosedEntry::Group {
                    window_id: WindowId::new(),
                    index: usize::MAX,
                    group: GroupSnapshot::of(&closed.group),
                    tabs: closed.tabs,
                },
            ));
        }
        let group = self.group(id)?.clone();
        let members = self.group_tabs(id);
        let index = members.first().and_then(|t| self.index_of(*t)).unwrap_or(0);
        let tabs = members
            .iter()
            .filter_map(|t| Some(TabSnapshot::of(self.tabs.get(t)?, self.extras.get(t))))
            .collect();
        let closed = self.close(&members, now);
        self.groups.remove(&id);
        Ok((
            closed,
            ClosedEntry::Group {
                window_id: group.window_id.unwrap_or_default(),
                index,
                group: GroupSnapshot::of(&group),
                tabs,
            },
        ))
    }

    /// Moves a whole group to the end of `window`, keeping it a group.
    pub fn move_group(&mut self, id: GroupId, window: WindowId, now: i64) -> Result<(), MadoError> {
        let group = self.group(id)?.clone();
        self.window(window)?;
        if group.window_id == Some(window) {
            return Ok(());
        }
        let tabs = self.group_tabs(id);
        let end = self.window(window)?.tab_order.len();
        self.move_tabs(&tabs, Some(window), end, Some(None), now)?;
        self.groups.insert(
            id,
            GroupState {
                window_id: Some(window),
                ..group
            },
        );
        for t in &tabs {
            if let Some(tab) = self.tabs.get_mut(t) {
                tab.group_id = Some(id);
            }
        }
        self.regroup(window);
        Ok(())
    }

    pub fn set_saved(&mut self, id: GroupId, saved: bool) -> Result<(), MadoError> {
        if let Some(c) = self.closed_groups.get_mut(&id) {
            c.group.saved = saved;
            if !saved {
                self.closed_groups.remove(&id);
            }
            return Ok(());
        }
        let g = self
            .groups
            .get_mut(&id)
            .ok_or_else(|| not_found("error.group.notFound"))?;
        if g.agent_task_id.is_some() && saved {
            return Err(invalid("error.group.agentGroup"));
        }
        g.saved = saved;
        Ok(())
    }

    // ----- the state document -----

    /// `state.savedGroups`: open and closed saved groups.
    pub fn saved_groups(&self) -> Vec<SavedGroupSummary> {
        let open = self
            .groups
            .values()
            .filter(|g| g.saved)
            .map(|g| SavedGroupSummary {
                id: g.id,
                title: g.title.clone(),
                color: g.color,
                tab_count: u32::try_from(self.group_tabs(g.id).len()).unwrap_or(u32::MAX),
                open: true,
            });
        let closed = self.closed_groups.values().map(|c| SavedGroupSummary {
            id: c.group.id,
            title: c.group.title.clone(),
            color: c.group.color,
            tab_count: u32::try_from(c.tabs.len()).unwrap_or(u32::MAX),
            open: false,
        });
        open.chain(closed).collect()
    }

    /// Writes what changed since `before` into the document.
    pub fn sync(&self, before: &Model, tx: &mut StatePatch<'_>) {
        let (put, gone) = diff(&before.windows, &self.windows);
        for (k, v) in put {
            tx.windows().insert(k, v);
        }
        for k in gone {
            tx.windows().remove(&k);
        }
        let (put, gone) = diff(&before.tabs, &self.tabs);
        for (k, v) in put {
            tx.tabs().insert(k, v);
        }
        for k in gone {
            tx.tabs().remove(&k);
        }
        let (put, gone) = diff(&before.groups, &self.groups);
        for (k, v) in put {
            tx.groups().insert(k, v);
        }
        for k in gone {
            tx.groups().remove(&k);
        }
        if before.saved_groups() != self.saved_groups() {
            tx.set_saved_groups(self.saved_groups());
        }
    }

    // ----- invariants -----

    /// The invariants of 04 §1. `Err` names the first one broken.
    pub fn check(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for w in self.windows.values() {
            let mut in_pinned = true;
            let mut finished: BTreeSet<GroupId> = BTreeSet::new();
            let mut current: Option<GroupId> = None;
            for t in &w.tab_order {
                if !seen.insert(*t) {
                    return Err(format!("tab {t} appears twice"));
                }
                let tab = self
                    .tabs
                    .get(t)
                    .ok_or_else(|| format!("unknown tab {t} in a window"))?;
                if tab.window_id != w.id {
                    return Err(format!("tab {t} lists another window"));
                }
                if tab.pinned && !in_pinned {
                    return Err(format!("pinned tab {t} after an unpinned one"));
                }
                in_pinned &= tab.pinned;
                if tab.pinned && tab.group_id.is_some() {
                    return Err(format!("pinned tab {t} is in a group"));
                }
                if let Some(g) = tab.group_id {
                    let group = self
                        .groups
                        .get(&g)
                        .ok_or_else(|| format!("tab {t} in a missing group"))?;
                    if group.window_id != Some(w.id) {
                        return Err(format!("group {g} is in another window than its tab"));
                    }
                    if current != Some(g) && finished.contains(&g) {
                        return Err(format!("group {g} is not contiguous"));
                    }
                }
                if current != tab.group_id
                    && let Some(c) = current
                {
                    finished.insert(c);
                }
                current = tab.group_id;
            }
            match w.active_tab_id {
                None if !w.tab_order.is_empty() => {
                    return Err(format!("window {} has no active tab", w.id));
                }
                Some(a) if !w.tab_order.contains(&a) => {
                    return Err(format!("window {}'s active tab is not in it", w.id));
                }
                Some(a)
                    if self
                        .tabs
                        .get(&a)
                        .is_some_and(|t| t.life == TabLife::Sleeping) =>
                {
                    return Err(format!("active tab {a} is sleeping"));
                }
                _ => {}
            }
        }
        if seen.len() != self.tabs.len() {
            return Err("a tab belongs to no window".into());
        }
        for g in self.groups.values() {
            if !self.tabs.values().any(|t| t.group_id == Some(g.id)) {
                return Err(format!("group {} has no tabs", g.id));
            }
        }
        Ok(())
    }
}

/// Entries added or changed, and keys removed.
fn diff<K: Ord + Copy, V: PartialEq + Clone>(
    before: &BTreeMap<K, V>,
    after: &BTreeMap<K, V>,
) -> (Vec<(K, V)>, Vec<K>) {
    let put = after
        .iter()
        .filter(|(k, v)| before.get(k) != Some(*v))
        .map(|(k, v)| (*k, v.clone()))
        .collect();
    let gone = before
        .keys()
        .filter(|k| !after.contains_key(k))
        .copied()
        .collect();
    (put, gone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const NOW: i64 = 1_000;

    fn model_with(windows: usize, tabs: usize) -> (Model, Vec<WindowId>) {
        let mut m = Model::default();
        let mut ids = Vec::new();
        for _ in 0..windows {
            let w = WindowId::new();
            m.add_window(w, Bounds::default(), TabLayout::Vertical, false);
            for i in 0..tabs {
                m.open_tab(
                    w,
                    format!("https://t{i}.example/"),
                    None,
                    i == 0,
                    None,
                    NOW,
                    Prefs::default(),
                )
                .expect("open");
            }
            ids.push(w);
        }
        (m, ids)
    }

    fn order(m: &Model, w: WindowId) -> Vec<String> {
        m.windows[&w]
            .tab_order
            .iter()
            .map(|t| {
                m.tabs[t]
                    .url
                    .trim_start_matches("https://")
                    .trim_end_matches(".example/")
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn links_open_after_the_opener_and_its_earlier_children() {
        let (mut m, w) = model_with(1, 3);
        let w = w[0];
        let opener = m.windows[&w].tab_order[0];
        for name in ["a", "b"] {
            m.open_tab(
                w,
                format!("https://{name}.example/"),
                None,
                false,
                Some(opener),
                NOW,
                Prefs::default(),
            )
            .expect("open");
        }
        assert_eq!(order(&m, w), ["t0", "a", "b", "t1", "t2"]);
        m.check().expect("invariants");
    }

    #[test]
    fn a_new_tab_never_goes_inside_the_pinned_block() {
        let (mut m, w) = model_with(1, 3);
        let w = w[0];
        let first = m.windows[&w].tab_order[0];
        let second = m.windows[&w].tab_order[1];
        m.pin(&[first, second], true).expect("pin");
        m.open_tab(
            w,
            "https://n.example/".into(),
            Some(TabPosition::Index { index: 0 }),
            false,
            None,
            NOW,
            Prefs::default(),
        )
        .expect("open");
        assert_eq!(order(&m, w), ["t0", "t1", "n", "t2"]);
        m.check().expect("invariants");
    }

    #[test]
    fn closing_the_active_tab_returns_to_its_opener_until_another_tab_is_chosen() {
        let (mut m, w) = model_with(1, 3);
        let w = w[0];
        let opener = m.windows[&w].tab_order[0];
        let child = m
            .open_tab(
                w,
                "https://c.example/".into(),
                None,
                true,
                Some(opener),
                NOW,
                Prefs::default(),
            )
            .expect("open");
        m.close(&[child], NOW);
        assert_eq!(m.windows[&w].active_tab_id, Some(opener));
        let child = m
            .open_tab(
                w,
                "https://c.example/".into(),
                None,
                true,
                Some(opener),
                NOW,
                Prefs::default(),
            )
            .expect("open");
        let other = m.windows[&w].tab_order[2];
        m.activate(other, NOW).expect("activate");
        m.activate(child, NOW).expect("activate");
        m.close(&[child], NOW);
        // Not the opener: the tab to the right.
        assert_eq!(m.windows[&w].active_tab_id, Some(other));
    }

    #[test]
    fn groups_stay_contiguous_and_a_drop_inside_joins() {
        let (mut m, w) = model_with(1, 5);
        let w = w[0];
        let o = m.windows[&w].tab_order.clone();
        let g = m
            .create_group(&[o[1], o[3]], Some("G".into()), None, NOW)
            .expect("group");
        assert_eq!(order(&m, w), ["t0", "t1", "t3", "t2", "t4"]);
        assert_eq!(m.groups[&g].color, GroupColor::Grey);
        // Dropping t4 between t1 and t3 joins the group.
        m.move_tabs(&[o[4]], None, 2, None, NOW).expect("move");
        assert_eq!(m.tabs[&o[4]].group_id, Some(g));
        // Dropping t1 at the end leaves it.
        m.move_tabs(&[o[1]], None, 4, None, NOW).expect("move");
        assert_eq!(m.tabs[&o[1]].group_id, None);
        m.check().expect("invariants");
        let g2 = m.create_group(&[o[0]], None, None, NOW).expect("group");
        assert_eq!(m.groups[&g2].color, GroupColor::Blue);
    }

    #[test]
    fn a_saved_group_closes_and_reopens_in_order() {
        let (mut m, w) = model_with(1, 4);
        let w = w[0];
        let o = m.windows[&w].tab_order.clone();
        let g = m
            .create_group(&[o[1], o[2]], Some("Work".into()), None, NOW)
            .expect("group");
        if let Some(t) = m.tabs.get_mut(&o[1]) {
            t.favicon = Some("mado-favicon://t1".into());
            t.title = "One".into();
        }
        let (closed, entry) = m.close_group(g, NOW).expect("close");
        assert!(entry.is_none());
        assert_eq!(closed.tabs.len(), 2);
        assert!(m.closed_groups.contains_key(&g));
        assert!(
            m.saved_groups()
                .iter()
                .any(|s| s.id == g && !s.open && s.tab_count == 2)
        );
        let ids = m.reopen_group(g, w, NOW).expect("reopen");
        assert_eq!(ids.len(), 2);
        assert_eq!(m.tabs[&ids[0]].url, "https://t1.example/");
        assert_eq!(m.tabs[&ids[0]].title, "One");
        assert_eq!(
            m.tabs[&ids[0]].favicon.as_deref(),
            Some("mado-favicon://t1")
        );
        assert_eq!(m.tabs[&ids[1]].life, TabLife::Sleeping);
        assert_eq!(m.windows[&w].active_tab_id, Some(ids[0]));
        m.check().expect("invariants");
    }

    #[derive(Debug, serde::Deserialize)]
    struct Scenario {
        name: String,
        tabs: Vec<String>,
        #[serde(with = "serde_norway::with::singleton_map_recursive")]
        steps: Vec<Step>,
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    enum Step {
        Open {
            name: String,
            #[serde(default)]
            opener: Option<String>,
            #[serde(default)]
            foreground: bool,
        },
        Activate(String),
        Close(Names),
        Pin(Vec<String>),
        Group(Vec<String>),
        Expect(String),
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(untagged)]
    enum Names {
        One(String),
        Many(Vec<String>),
    }

    /// 04 §12 check 2.
    #[test]
    fn selection_matches_chrome() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/tabs/selection.yaml"
        );
        let text = std::fs::read_to_string(path).expect("tests/tabs/selection.yaml");
        let scenarios: Vec<Scenario> = serde_norway::from_str(&text).expect("valid scenarios");
        assert!(scenarios.len() >= 10);
        for s in scenarios {
            let mut m = Model::default();
            let w = WindowId::new();
            m.add_window(w, Bounds::default(), TabLayout::Vertical, false);
            let mut ids: BTreeMap<String, TabId> = BTreeMap::new();
            for (i, name) in s.tabs.iter().enumerate() {
                let id = m
                    .open_tab(
                        w,
                        format!("https://{name}.example/"),
                        None,
                        i == 0,
                        None,
                        NOW,
                        Prefs::default(),
                    )
                    .expect("open");
                ids.insert(name.clone(), id);
            }
            for step in &s.steps {
                let id = |n: &String| {
                    ids.get(n)
                        .copied()
                        .unwrap_or_else(|| panic!("{}: no tab {n}", s.name))
                };
                match step {
                    Step::Open {
                        name,
                        opener,
                        foreground,
                    } => {
                        let t = m
                            .open_tab(
                                w,
                                format!("https://{name}.example/"),
                                None,
                                *foreground,
                                opener.as_ref().map(id),
                                NOW,
                                Prefs::default(),
                            )
                            .expect("open");
                        ids.insert(name.clone(), t);
                    }
                    Step::Activate(n) => {
                        m.activate(id(n), NOW).expect("activate");
                    }
                    Step::Close(Names::One(n)) => {
                        m.close(&[id(n)], NOW);
                    }
                    Step::Close(Names::Many(ns)) => {
                        m.close(&ns.iter().map(id).collect::<Vec<_>>(), NOW);
                    }
                    Step::Pin(ns) => m
                        .pin(&ns.iter().map(id).collect::<Vec<_>>(), true)
                        .expect("pin"),
                    Step::Group(ns) => {
                        m.create_group(&ns.iter().map(id).collect::<Vec<_>>(), None, None, NOW)
                            .expect("group");
                    }
                    Step::Expect(n) => {
                        assert_eq!(
                            m.windows[&w].active_tab_id,
                            Some(id(n)),
                            "{}: expected {n}",
                            s.name
                        );
                    }
                }
                m.check().unwrap_or_else(|e| panic!("{}: {e}", s.name));
            }
        }
    }

    #[derive(Debug, Clone)]
    enum Op {
        Open {
            w: usize,
            pos: Option<u32>,
            fg: bool,
            opener: Option<usize>,
        },
        Close(Vec<usize>),
        Activate(usize),
        Move {
            tabs: Vec<usize>,
            w: Option<usize>,
            index: usize,
            group: Option<Option<usize>>,
        },
        Pin(Vec<usize>, bool),
        Group(Vec<usize>),
        Ungroup(usize),
        CloseGroup(usize),
        Reopen(usize, usize),
        Collapse(usize),
        Next(usize, bool),
        NewWindow,
        Unload(usize),
    }

    fn op() -> impl Strategy<Value = Op> {
        let ix = || 0usize..12;
        let few = || prop::collection::vec(0usize..12, 1..4);
        prop_oneof![
            (
                ix(),
                prop::option::of(0u32..12),
                any::<bool>(),
                prop::option::of(ix())
            )
                .prop_map(|(w, pos, fg, opener)| Op::Open { w, pos, fg, opener }),
            few().prop_map(Op::Close),
            ix().prop_map(Op::Activate),
            (
                few(),
                prop::option::of(ix()),
                0usize..12,
                prop::option::of(prop::option::of(ix()))
            )
                .prop_map(|(tabs, w, index, group)| Op::Move {
                    tabs,
                    w,
                    index,
                    group
                }),
            (few(), any::<bool>()).prop_map(|(t, p)| Op::Pin(t, p)),
            few().prop_map(Op::Group),
            ix().prop_map(Op::Ungroup),
            ix().prop_map(Op::CloseGroup),
            (ix(), ix()).prop_map(|(g, w)| Op::Reopen(g, w)),
            ix().prop_map(Op::Collapse),
            (ix(), any::<bool>()).prop_map(|(w, f)| Op::Next(w, f)),
            Just(Op::NewWindow),
            ix().prop_map(Op::Unload),
        ]
    }

    fn pick<T: Copy>(items: &[T], i: usize) -> Option<T> {
        (!items.is_empty()).then(|| items[i % items.len()])
    }

    fn run(m: &mut Model, op: Op) {
        let tabs: Vec<TabId> = m.tabs.keys().copied().collect();
        let windows: Vec<WindowId> = m.windows.keys().copied().collect();
        let groups: Vec<GroupId> = m.groups.keys().copied().collect();
        let closed: Vec<GroupId> = m.closed_groups.keys().copied().collect();
        let many =
            |ix: &[usize]| -> Vec<TabId> { ix.iter().filter_map(|i| pick(&tabs, *i)).collect() };
        let prefs = Prefs::default();
        let _ = match op {
            Op::Open { w, pos, fg, opener } => pick(&windows, w).map(|w| {
                let _ = m.open_tab(
                    w,
                    "https://x.example/".into(),
                    pos.map(|index| TabPosition::Index { index }),
                    fg,
                    opener.and_then(|o| pick(&tabs, o)),
                    NOW,
                    prefs,
                );
            }),
            Op::Close(t) => {
                m.close(&many(&t), NOW);
                Some(())
            }
            Op::Activate(t) => pick(&tabs, t).map(|t| {
                let _ = m.activate(t, NOW);
            }),
            Op::Move {
                tabs: t,
                w,
                index,
                group,
            } => {
                let _ = m.move_tabs(
                    &many(&t),
                    w.and_then(|w| pick(&windows, w)),
                    index,
                    group.map(|g| g.and_then(|g| pick(&groups, g))),
                    NOW,
                );
                Some(())
            }
            Op::Pin(t, p) => {
                let _ = m.pin(&many(&t), p);
                Some(())
            }
            Op::Group(t) => {
                let _ = m.create_group(&many(&t), None, None, NOW);
                Some(())
            }
            Op::Ungroup(g) => pick(&groups, g).map(|g| {
                let _ = m.ungroup(g);
            }),
            Op::CloseGroup(g) => pick(&groups, g).map(|g| {
                let _ = m.close_group(g, NOW);
            }),
            Op::Reopen(g, w) => pick(&closed, g).zip(pick(&windows, w)).map(|(g, w)| {
                let _ = m.reopen_group(g, w, NOW);
            }),
            Op::Collapse(g) => pick(&groups, g).map(|g| {
                let _ = m.update_group(g, None, None, Some(true));
            }),
            Op::Next(w, f) => pick(&windows, w).map(|w| {
                if let Ok(Some(t)) = m.neighbour(w, f, prefs) {
                    let _ = m.activate(t, NOW);
                }
            }),
            Op::NewWindow => {
                let w = WindowId::new();
                m.add_window(w, Bounds::default(), TabLayout::Vertical, false);
                let _ = m.open_tab(w, "https://n.example/".into(), None, true, None, NOW, prefs);
                Some(())
            }
            Op::Unload(t) => pick(&tabs, t).map(|t| {
                if m.tabs.get(&t).is_some_and(|t| t.pinned) {
                    let _ = m.unload_pinned(t, NOW);
                }
            }),
        };
    }

    proptest! {
        // 04 §12 check 1: 10,000 random command sequences keep every
        // invariant of 04 §1.
        #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]
        #[test]
        fn random_command_sequences_keep_every_invariant(ops in prop::collection::vec(op(), 1..40)) {
            let (mut m, _) = model_with(2, 3);
            for op in ops {
                let shown = format!("{op:?}");
                run(&mut m, op);
                if let Err(e) = m.check() {
                    return Err(TestCaseError::fail(format!("after {shown}: {e}")));
                }
            }
        }
    }
}
