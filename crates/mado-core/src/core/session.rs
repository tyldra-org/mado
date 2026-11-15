//! Windows, tabs, groups and sessions in the core (spec 04): the commands,
//! the engine events that change tabs, the closed stack, sleeping, startup
//! and the quit flow. The tab model itself is `crate::tabs`.

use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;
use mado_engine::event::{EngineSecurity, GoneReason};
use mado_engine::tab::{Image, ImageFormat, PopupDecision, PopupRequest, SessionBlob};
use mado_engine::{EngineEvent, EngineOp, TabCommand, TabInitial, TabSpec, WindowCommand};
use mado_types::command::{Command, NamedPosition, Origin, TabPosition};
use mado_types::state::{
    Bounds, Notice, NoticeAction, PromptState, SecurityLevel, TabLayout, TabLife, TabState,
    UnsavedTab, WindowKind,
};
use mado_types::{ErrorCode, GroupId, MadoError, PatchRecorder, PromptId, TabId, WindowId};
use serde_json::{Value, json};

use super::{Core, Internal, lookup_def};
use crate::store;
use crate::tabs::{ClosedEntry, Model, Prefs, SessionWindow, TabSnapshot};
use crate::timers::Timer;

/// The New Tab page (spec 04 §3).
pub const NEW_TAB: &str = "mado://newtab";
/// How often the core looks for tabs to sleep (spec 04 §7).
pub const SLEEP_CHECK: Duration = Duration::from_secs(60);
/// Maximum mode's inactive time.
const MAXIMUM_AFTER: i64 = 10 * 60 * 1000;
/// Moderate mode sleeps only tabs idle this long under memory pressure.
const MODERATE_PRESSURE_IDLE: i64 = 5 * 60 * 1000;
/// Two ⌘Q presses within this quit (spec 04 §9).
const QUIT_AGAIN: i64 = 1000;

/// What a pending unsaved-text prompt closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CloseAction {
    Quit,
    Window(WindowId),
}

/// The power source, for energy saver (spec 04 §7).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Power {
    pub on_battery: bool,
    /// 0 to 100.
    pub percent: u8,
}

fn not_found(key: &str) -> MadoError {
    MadoError::new(ErrorCode::NotFound, key)
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn security_of(url: &str, engine: Option<&EngineSecurity>) -> SecurityLevel {
    // `user:pass@` is never shown, so the icon warns instead (05 §4).
    if url::Url::parse(url).is_ok_and(|u| !u.username().is_empty() || u.password().is_some()) {
        return SecurityLevel::NotSecure;
    }
    match url
        .split_once(':')
        .map(|(s, _)| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("https") => match engine {
            Some(e) if e.mixed_content => SecurityLevel::Mixed,
            Some(e) if !e.secure => SecurityLevel::Dangerous,
            _ => SecurityLevel::Secure,
        },
        Some("http") => SecurityLevel::NotSecure,
        Some("file") => SecurityLevel::Local,
        _ => SecurityLevel::Internal,
    }
}

impl Core {
    fn setting(&self, key: &str) -> Value {
        self.settings.value(lookup_def(key))
    }

    fn setting_str(&self, key: &str) -> String {
        self.setting(key).as_str().unwrap_or_default().to_owned()
    }

    pub(super) fn prefs(&self) -> Prefs {
        Prefs {
            new_tab_after_current: self.setting_str("tabs.newTabPosition") == "afterCurrent",
            ctrl_tab_recent: self.setting_str("tabs.ctrlTabOrder") == "recent",
        }
    }

    fn layout(&self) -> TabLayout {
        if self.setting_str("appearance.layout") == "horizontal" {
            TabLayout::Horizontal
        } else {
            TabLayout::Vertical
        }
    }

    /// The window the user worked in last: the one holding the most
    /// recently active tab.
    pub(super) fn current_window(&self) -> Option<WindowId> {
        self.model
            .windows
            .values()
            .filter(|w| w.kind == WindowKind::Normal)
            .max_by_key(|w| {
                w.active_tab_id
                    .and_then(|t| self.model.tabs.get(&t))
                    .map_or(0, |t| t.last_active_at)
            })
            .map(|w| w.id)
    }

    /// A normal window with one tab per URL (a New Tab page if none).
    pub(super) fn open_window(
        &mut self,
        bounds: Bounds,
        urls: &[String],
    ) -> Result<WindowId, MadoError> {
        let id = WindowId::new();
        let private = self.state.profile.is_private;
        self.model.add_window(id, bounds, self.layout(), private);
        let now = self.now();
        let prefs = self.prefs();
        let urls: Vec<String> = if urls.is_empty() {
            vec![NEW_TAB.to_owned()]
        } else {
            urls.to_vec()
        };
        for (i, url) in urls.into_iter().enumerate() {
            self.model.open_tab(
                id,
                url,
                Some(TabPosition::Named(NamedPosition::End)),
                i == 0,
                None,
                now,
                prefs,
            )?;
        }
        Ok(id)
    }

    // ----- commands -----

    pub(super) fn tab_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let now = self.now();
        let prefs = self.prefs();
        match cmd {
            Command::WindowReportContentRect {
                window_id, rect, ..
            } => {
                if !matches!(origin, Origin::Ui { window } if *window == window_id) {
                    return Err(MadoError::new(
                        ErrorCode::PermissionDenied,
                        "error.window.notYours",
                    ));
                }
                let w = self
                    .model
                    .windows
                    .get_mut(&window_id)
                    .ok_or_else(|| not_found("error.window.notFound"))?;
                w.content_rect = Some(rect);
                Ok(Value::Null)
            }
            Command::WindowNew { private, urls } => {
                if private && !self.state.profile.is_private {
                    // Private windows run in their own temporary profile
                    // process, which the profiles task adds (M2-T09).
                    return Err(MadoError::new(
                        ErrorCode::EngineUnavailable,
                        "error.window.privateUnavailable",
                    ));
                }
                let id = self.open_window(Bounds::default(), &urls)?;
                Ok(json!({ "windowId": id }))
            }
            Command::WindowClose { window_id } => {
                self.model.window(window_id)?;
                self.request_close(CloseAction::Window(window_id), rec);
                Ok(Value::Null)
            }
            Command::WindowSetLayout { layout, .. } => {
                let value = match layout {
                    TabLayout::Vertical => "vertical",
                    TabLayout::Horizontal => "horizontal",
                };
                let out = self.settings.set(
                    "appearance.layout",
                    json!(value),
                    origin,
                    None,
                    self.tokens.at(now),
                    now,
                )?;
                self.apply_outcome(out, rec);
                for w in self.model.windows.values_mut() {
                    w.layout = layout;
                }
                Ok(Value::Null)
            }
            Command::TabNew {
                window_id,
                url,
                position,
                background,
                opener_id,
            } => {
                let url = url.unwrap_or_else(|| NEW_TAB.to_owned());
                let id = self.model.open_tab(
                    window_id,
                    url,
                    position,
                    !background,
                    opener_id,
                    now,
                    prefs,
                )?;
                Ok(json!({ "tabId": id }))
            }
            Command::TabClose { tab_ids } => {
                for t in &tab_ids {
                    self.model.tab(*t)?;
                }
                self.close_tabs(&tab_ids, true);
                Ok(Value::Null)
            }
            Command::TabActivate { tab_id } => {
                self.model.activate(tab_id, now)?;
                Ok(Value::Null)
            }
            Command::TabMove {
                tab_ids,
                to_window_id,
                to_index,
                to_group_id,
            } => {
                let index = usize::try_from(to_index).unwrap_or(usize::MAX);
                self.model
                    .move_tabs(&tab_ids, to_window_id, index, to_group_id, now)?;
                Ok(Value::Null)
            }
            Command::TabToNewWindow { tab_ids, bounds } => {
                for t in &tab_ids {
                    self.model.tab(*t)?;
                }
                let id = WindowId::new();
                let private = self.state.profile.is_private;
                self.model
                    .add_window(id, bounds.unwrap_or_default(), self.layout(), private);
                self.model
                    .move_tabs(&tab_ids, Some(id), 0, Some(None), now)?;
                Ok(json!({ "windowId": id }))
            }
            Command::TabPin { tab_ids } => self.model.pin(&tab_ids, true).map(|()| Value::Null),
            Command::TabUnpin { tab_ids } => self.model.pin(&tab_ids, false).map(|()| Value::Null),
            Command::TabResetPin { tab_id } => {
                let home = self
                    .model
                    .extras
                    .get(&tab_id)
                    .and_then(|e| e.pinned_home_url.clone())
                    .ok_or_else(|| {
                        MadoError::new(ErrorCode::InvalidInput, "error.tab.notPinned")
                    })?;
                self.navigate(tab_id, home)?;
                Ok(Value::Null)
            }
            Command::TabRename { tab_id, title } => {
                self.model.rename(tab_id, &title).map(|()| Value::Null)
            }
            Command::TabDuplicate { tab_id } => {
                let source = self.model.tab(tab_id)?.clone();
                let index = self
                    .model
                    .window(source.window_id)?
                    .tab_order
                    .iter()
                    .position(|t| *t == tab_id)
                    .unwrap_or(0);
                let id = self.model.open_tab(
                    source.window_id,
                    source.url.clone(),
                    Some(TabPosition::Index {
                        index: u32::try_from(index + 1).unwrap_or(u32::MAX),
                    }),
                    true,
                    None,
                    now,
                    prefs,
                )?;
                let session = self
                    .model
                    .extras
                    .get(&tab_id)
                    .and_then(|e| e.session.clone());
                if let Some(t) = self.model.tabs.get_mut(&id) {
                    t.title = source.title;
                    t.favicon = source.favicon;
                    t.zoom = source.zoom;
                }
                if let Some(e) = self.model.extras.get_mut(&id) {
                    e.session = session;
                }
                Ok(json!({ "tabId": id }))
            }
            Command::TabSleep { tab_ids } => {
                for t in &tab_ids {
                    self.model.tab(*t)?;
                }
                let keep_awake = self.keep_awake_sites();
                for t in tab_ids {
                    if self.can_sleep(t, &keep_awake) {
                        self.sleep_tab(t);
                    }
                }
                Ok(Value::Null)
            }
            Command::TabMute { tab_ids, muted } => {
                for t in &tab_ids {
                    self.model.tab(*t)?;
                }
                for t in tab_ids {
                    if let Some(tab) = self.model.tabs.get_mut(&t) {
                        tab.audio.muted = muted;
                    }
                }
                Ok(Value::Null)
            }
            Command::TabMuteSite { host, muted } => {
                let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
                if host.is_empty() || host.len() > 253 || host.contains(['/', ' ', ':']) {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.site.badHost",
                    ));
                }
                for t in self.model.tabs.values_mut() {
                    if host_of(&t.url) == host {
                        t.audio.muted = muted;
                    }
                }
                if !self.state.profile.is_private {
                    let origin_pattern = format!("[*.]{host}");
                    let value = if muted { "block" } else { "allow" };
                    let pending = self.profile_db.writer().write(move |c| {
                        c.execute(
                            "INSERT INTO site_settings (origin, key, value, embedder, granted_at, source)
                             VALUES (?1, 'sound', ?2, '', ?3, 'user')
                             ON CONFLICT DO UPDATE SET value = ?2, granted_at = ?3, source = 'user'",
                            rusqlite::params![origin_pattern, value, now],
                        )
                    });
                    tokio::task::spawn_local(async move {
                        if let Err(e) = pending.await {
                            tracing::error!(error = %e, "the sound site setting was not written");
                        }
                    });
                }
                Ok(Value::Null)
            }
            Command::TabMoveToProfile { tab_id, profile_id } => {
                if profile_id == self.state.profile.id {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.profile.same",
                    ));
                }
                let url = self.model.tab(tab_id)?.url.clone();
                self.ui.open_in_profile(profile_id, &url);
                self.close_tabs(&[tab_id], false);
                Ok(Value::Null)
            }
            Command::TabSelectNext { window_id } | Command::TabSelectPrevious { window_id } => {
                let forward = matches!(cmd, Command::TabSelectNext { .. });
                if let Some(t) = self.model.neighbour(window_id, forward, prefs)? {
                    self.model.activate(t, now)?;
                }
                Ok(Value::Null)
            }
            Command::TabSelectIndex {
                window_id,
                index,
                last,
            } => {
                let index = usize::try_from(index).unwrap_or(usize::MAX);
                if let Some(t) = self.model.at_index(window_id, index, last)? {
                    self.model.activate(t, now)?;
                }
                Ok(Value::Null)
            }
            Command::TabCloseOthers { tab_id } | Command::TabCloseRight { tab_id } => {
                let right = matches!(cmd, Command::TabCloseRight { .. });
                let others = self.model.others(tab_id, right)?;
                self.model.activate(tab_id, now)?;
                self.close_tabs(&others, false);
                Ok(Value::Null)
            }
            Command::TabReloadAll { window_id } => {
                let order = self.model.window(window_id)?.tab_order.clone();
                for t in order {
                    if self
                        .model
                        .tabs
                        .get(&t)
                        .is_some_and(|t| t.life != TabLife::Sleeping)
                    {
                        self.outbox.push(EngineOp::Apply(
                            t,
                            TabCommand::Reload {
                                bypass_cache: false,
                            },
                        ));
                    }
                }
                Ok(Value::Null)
            }
            cmd @ (Command::TabReload { .. }
            | Command::TabStop { .. }
            | Command::TabBack { .. }
            | Command::TabForward { .. }
            | Command::TabZoom { .. }
            | Command::TabFind { .. }
            | Command::TabPrint { .. }
            | Command::TabSave { .. }
            | Command::TabAnswerUnresponsive { .. }) => self.page_command(cmd, rec),
            Command::TabHoverInfo { tab_id } => {
                let t = self.model.tab(tab_id)?;
                let mut info = json!({
                    "title": t.custom_title.as_ref().unwrap_or(&t.title),
                    "url": t.url,
                    "sleeping": t.life == TabLife::Sleeping,
                });
                // Memory comes from the tab's WebContent process through the
                // adapter; absent until it reports (spec 04 §11).
                if let Some(snap) = self
                    .model
                    .extras
                    .get(&tab_id)
                    .and_then(|e| e.snapshot.as_ref())
                {
                    info["snapshot"] = json!(format!(
                        "data:image/jpeg;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(snap)
                    ));
                }
                Ok(info)
            }
            Command::GroupCreate {
                tab_ids,
                title,
                color,
            } => {
                let id = self.model.create_group(&tab_ids, title, color, now)?;
                Ok(json!({ "groupId": id }))
            }
            Command::GroupUpdate {
                group_id,
                title,
                color,
                collapsed,
            } => self
                .model
                .update_group(group_id, title, color, collapsed)
                .map(|()| Value::Null),
            Command::GroupUngroup { group_id } => {
                self.model.ungroup(group_id).map(|()| Value::Null)
            }
            Command::GroupClose { group_id } => {
                let (_, entry) = self.model.close_group(group_id, now)?;
                if let Some(entry) = entry {
                    self.push_closed(entry);
                }
                Ok(Value::Null)
            }
            Command::GroupReopen {
                group_id,
                window_id,
            } => {
                self.model.reopen_group(group_id, window_id, now)?;
                Ok(Value::Null)
            }
            Command::GroupDelete { group_id } => {
                let (_, entry) = self.model.delete_group(group_id, now)?;
                self.deleted_groups.insert(group_id, entry);
                let id = format!("n{}", self.next_notice);
                self.next_notice += 1;
                let undo = Command::GroupUndoDelete { group_id };
                let notice = Notice {
                    id: id.clone(),
                    message_key: "notice.groupDeleted".into(),
                    args: BTreeMap::new(),
                    action: Some(NoticeAction {
                        label_key: "notice.undo".into(),
                        command: undo.clone(),
                    }),
                    undo: Some(undo),
                    expires_at: Some(
                        now + i64::try_from(super::UNDO_NOTICE.as_millis()).unwrap_or(10_000),
                    ),
                };
                self.add_notice(notice, rec);
                self.timers
                    .schedule(Timer::NoticeExpiry(id), super::UNDO_NOTICE);
                self.timers
                    .schedule(Timer::ForgetDeletedGroup(group_id), super::UNDO_NOTICE);
                Ok(Value::Null)
            }
            Command::GroupUndoDelete { group_id } => {
                let entry = self
                    .deleted_groups
                    .remove(&group_id)
                    .ok_or_else(|| not_found("error.group.undoExpired"))?;
                self.restore_entry(entry)?;
                Ok(Value::Null)
            }
            Command::GroupSetSaved { group_id, saved } => {
                self.model.set_saved(group_id, saved).map(|()| Value::Null)
            }
            Command::GroupMoveToWindow {
                group_id,
                window_id,
            } => {
                self.model.group(group_id)?;
                let target = match window_id {
                    Some(w) => {
                        self.model.window(w)?;
                        w
                    }
                    None => {
                        let id = WindowId::new();
                        let private = self.state.profile.is_private;
                        self.model
                            .add_window(id, Bounds::default(), self.layout(), private);
                        id
                    }
                };
                self.model.move_group(group_id, target, now)?;
                Ok(json!({ "windowId": target }))
            }
            Command::SessionReopenClosed => {
                let (id, entry) = self
                    .closed
                    .pop()
                    .ok_or_else(|| not_found("error.session.nothingClosed"))?;
                if !self.state.profile.is_private {
                    let pending = self
                        .profile_db
                        .writer()
                        .write(move |c| store::pop_closed(c, id));
                    tokio::task::spawn_local(async move {
                        if let Err(e) = pending.await {
                            tracing::error!(error = %e, "the closed stack was not updated");
                        }
                    });
                }
                self.restore_entry(entry)?;
                Ok(Value::Null)
            }
            Command::SessionRecentlyClosed => Ok(json!(
                self.closed
                    .iter()
                    .rev()
                    .take(25)
                    .map(|(_, e)| e.summary())
                    .collect::<Vec<_>>()
            )),
            Command::SessionConfirmClose { prompt_id, proceed } => {
                let action = self
                    .close_prompts
                    .remove(&prompt_id)
                    .ok_or_else(|| not_found("error.prompt.notFound"))?;
                let mut prompts = self.state.prompts.clone();
                prompts.remove(&prompt_id);
                self.state.tx(rec).set_prompts(prompts);
                if proceed {
                    self.finish_close(action);
                }
                Ok(Value::Null)
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                Err(MadoError::internal())
            }
        }
    }

    fn navigate(&mut self, tab: TabId, url: String) -> Result<(), MadoError> {
        let t = self
            .model
            .tabs
            .get_mut(&tab)
            .ok_or_else(|| not_found("error.tab.notFound"))?;
        if t.life == TabLife::Sleeping {
            t.url = url;
            if let Some(e) = self.model.extras.get_mut(&tab) {
                e.session = None;
            }
        } else {
            self.outbox.push(EngineOp::Apply(
                tab,
                TabCommand::Navigate {
                    url,
                    referrer: None,
                    kind: mado_engine::NavKind::Other,
                },
            ));
        }
        Ok(())
    }

    pub(super) fn add_notice(&mut self, notice: Notice, rec: &mut PatchRecorder) {
        let mut notices = self.state.notices.clone();
        notices.push(notice.clone());
        self.state.tx(rec).set_notices(notices);
        self.ui.notice(&notice);
    }

    // ----- closing -----

    /// Closes tabs, pushing one closed-stack entry (04 §4). A single active
    /// pinned tab closed by the user follows `tabs.closePinned`.
    fn close_tabs(&mut self, tabs: &[TabId], user_close: bool) {
        let now = self.now();
        if user_close
            && let [one] = tabs
            && self.model.tabs.get(one).is_some_and(|t| t.pinned)
            && self.setting_str("tabs.closePinned") != "close"
            && self.model.unload_pinned(*one, now).unwrap_or(false)
        {
            return;
        }
        let closed = self.model.close(tabs, now);
        let window_of = closed.tabs.first().map(|r| r.tab.window_id);
        let entry = match closed.tabs.as_slice() {
            [] => None,
            [one] => Some(ClosedEntry::Tab {
                window_id: one.tab.window_id,
                index: one.index,
                group_id: one.tab.group_id,
                tab: snapshot(&one.tab, &one.extra),
            }),
            many => Some(ClosedEntry::Tabs {
                window_id: window_of.unwrap_or_default(),
                tabs: many
                    .iter()
                    .map(|r| (r.index, r.tab.group_id, snapshot(&r.tab, &r.extra)))
                    .collect(),
            }),
        };
        if let Some(entry) = entry {
            self.push_closed(entry);
        }
    }

    fn push_closed(&mut self, entry: ClosedEntry) {
        let id = self.closed.last().map_or(1, |(i, _)| i + 1);
        if !self.state.profile.is_private {
            let now = self.now();
            let stored = entry.clone();
            let pending = self
                .profile_db
                .writer()
                .write(move |c| store::push_closed(c, id, &stored, now));
            tokio::task::spawn_local(async move {
                if let Err(e) = pending.await {
                    tracing::error!(error = %e, "the closed stack was not written");
                }
            });
        }
        self.closed.push((id, entry));
        if self.closed.len() > 100 {
            self.closed.remove(0);
        }
    }

    /// Puts a closed-stack entry back (04 §6). Restored tabs sleep except
    /// the one that becomes active.
    pub(super) fn restore_entry(&mut self, entry: ClosedEntry) -> Result<(), MadoError> {
        let now = self.now();
        let target = |core: &mut Self, wanted: WindowId| -> Result<WindowId, MadoError> {
            if core.model.windows.contains_key(&wanted) {
                return Ok(wanted);
            }
            if let Some(w) = core.current_window() {
                return Ok(w);
            }
            let id = WindowId::new();
            let private = core.state.profile.is_private;
            core.model
                .add_window(id, Bounds::default(), core.layout(), private);
            Ok(id)
        };
        match entry {
            ClosedEntry::Tab {
                window_id,
                index,
                group_id,
                tab,
            } => {
                let w = target(self, window_id)?;
                let id = self.model.restore_tab(w, index, group_id, &tab, now)?;
                self.model.activate(id, now)?;
            }
            ClosedEntry::Tabs { window_id, tabs } => {
                let w = target(self, window_id)?;
                let mut last = None;
                for (index, group, tab) in &tabs {
                    last = Some(self.model.restore_tab(w, *index, *group, tab, now)?);
                }
                if let Some(t) = last {
                    self.model.activate(t, now)?;
                }
            }
            ClosedEntry::Group {
                window_id,
                index,
                group,
                tabs,
            } => {
                let w = target(self, window_id)?;
                let index = index.min(self.model.window(w)?.tab_order.len());
                let ids = self.model.restore_group(w, index, &group, &tabs, now)?;
                if let Some(first) = ids.first() {
                    self.model.activate(*first, now)?;
                }
            }
            ClosedEntry::Window {
                window,
                tabs,
                groups,
            } => {
                self.restore_window(
                    &SessionWindow {
                        window,
                        tabs,
                        groups,
                    },
                    now,
                )?;
            }
            ClosedEntry::Session { windows } => {
                for w in &windows {
                    self.restore_window(w, now)?;
                }
            }
        }
        Ok(())
    }

    fn restore_window(&mut self, saved: &SessionWindow, now: i64) -> Result<WindowId, MadoError> {
        if saved.tabs.is_empty() {
            return Err(not_found("error.session.emptyWindow"));
        }
        let id = WindowId::new();
        let private = self.state.profile.is_private;
        self.model.add_window(
            id,
            saved.window.bounds.clone(),
            saved.window.layout,
            private,
        );
        if let Some(w) = self.model.windows.get_mut(&id) {
            w.state = saved.window.state;
            w.sidebar.width = saved.window.sidebar_width;
        }
        let mut groups: BTreeMap<GroupId, GroupId> = BTreeMap::new();
        let mut ids = Vec::new();
        for (i, t) in saved.tabs.iter().enumerate() {
            let group = match t.group {
                Some(old) => match groups.get(&old) {
                    Some(new) => Some(*new),
                    None => saved.groups.iter().find(|g| g.id == old).map(|g| {
                        let new = if self.model.groups.contains_key(&g.id)
                            || self.model.closed_groups.contains_key(&g.id)
                        {
                            GroupId::new()
                        } else {
                            g.id
                        };
                        self.model.groups.insert(
                            new,
                            mado_types::state::GroupState {
                                id: new,
                                window_id: Some(id),
                                title: g.title.clone(),
                                color: g.color,
                                collapsed: g.collapsed,
                                saved: g.saved,
                                agent_task_id: None,
                            },
                        );
                        groups.insert(old, new);
                        new
                    }),
                },
                None => None,
            };
            ids.push(self.model.restore_tab(id, i, group, &t.tab, now)?);
        }
        let active = saved
            .window
            .active
            .and_then(|i| ids.get(i))
            .or_else(|| ids.first())
            .copied();
        if let Some(a) = active {
            self.model.activate(a, now)?;
        }
        let gone: Vec<GroupId> = self
            .model
            .groups
            .keys()
            .filter(|g| !self.model.tabs.values().any(|t| t.group_id == Some(**g)))
            .copied()
            .collect();
        for g in gone {
            self.model.groups.remove(&g);
        }
        Ok(id)
    }

    // ----- quit and unsaved text (04 §9) -----

    /// ⌘Q, the Dock's Quit or a window's close button. With
    /// `general.warnBeforeQuit`, ⌘Q quits only when pressed twice within a
    /// second.
    pub(super) fn quit_requested(&mut self, shortcut: bool, rec: &mut PatchRecorder) {
        let now = self.now();
        if shortcut && self.setting("general.warnBeforeQuit") == true {
            let again = self
                .quit_pressed_at
                .is_some_and(|at| now - at <= QUIT_AGAIN);
            if !again {
                self.quit_pressed_at = Some(now);
                let id = format!("n{}", self.next_notice);
                self.next_notice += 1;
                let notice = Notice {
                    id: id.clone(),
                    message_key: "notice.pressAgainToQuit".into(),
                    args: BTreeMap::new(),
                    action: None,
                    undo: None,
                    expires_at: Some(now + 2000),
                };
                self.add_notice(notice, rec);
                self.timers
                    .schedule(Timer::NoticeExpiry(id), Duration::from_secs(2));
                return;
            }
        }
        self.quit_pressed_at = None;
        self.request_close(CloseAction::Quit, rec);
    }

    /// The system asked Mado to stop: no prompts, downloads pause first.
    pub(super) fn terminate(&mut self) {
        self.finish_close(CloseAction::Quit);
    }

    fn request_close(&mut self, action: CloseAction, rec: &mut PatchRecorder) {
        let unsaved: Vec<UnsavedTab> = self
            .model
            .tabs
            .values()
            .filter(|t| t.has_unsaved_input)
            .filter(|t| match action {
                CloseAction::Quit => true,
                CloseAction::Window(w) => t.window_id == w,
            })
            .map(|t| UnsavedTab {
                tab_id: t.id,
                title: t.custom_title.clone().unwrap_or_else(|| t.title.clone()),
                host: host_of(&t.url),
            })
            .collect();
        if unsaved.is_empty() || self.setting("general.warnUnsavedText") != true {
            self.finish_close(action);
            return;
        }
        if self.close_prompts.values().any(|a| *a == action) {
            return;
        }
        let id = PromptId::new();
        self.close_prompts.insert(id, action);
        let mut prompts = self.state.prompts.clone();
        prompts.insert(
            id,
            PromptState::UnsavedText {
                id,
                window_id: match action {
                    CloseAction::Quit => None,
                    CloseAction::Window(w) => Some(w),
                },
                tabs: unsaved,
            },
        );
        self.state.tx(rec).set_prompts(prompts);
    }

    /// Pages' own `beforeunload` handlers run with page dialogs (M2-T07).
    fn finish_close(&mut self, action: CloseAction) {
        match action {
            CloseAction::Quit => {
                if self.pause_downloads_for_quit() {
                    self.timers
                        .schedule(Timer::QuitNow, std::time::Duration::from_secs(3));
                } else {
                    self.ui.quit();
                }
            }
            CloseAction::Window(w) => {
                if let Ok((snapshot, _)) = self.model.remove_window(w)
                    && !snapshot.tabs.is_empty()
                {
                    self.push_closed(ClosedEntry::Window {
                        window: snapshot.window,
                        tabs: snapshot.tabs,
                        groups: snapshot.groups,
                    });
                }
            }
        }
    }

    // ----- sleeping (04 §7) -----

    fn keep_awake_sites(&self) -> Vec<String> {
        self.setting("tabs.keepAwakeSites")
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether `tab` is a sleep candidate.
    fn can_sleep(&self, tab: TabId, keep_awake: &[String]) -> bool {
        let Some(t) = self.model.tabs.get(&tab) else {
            return false;
        };
        let active = self
            .model
            .windows
            .get(&t.window_id)
            .is_some_and(|w| w.active_tab_id == Some(tab));
        let host = host_of(&t.url);
        let awake_site = keep_awake
            .iter()
            .any(|s| host == *s || host.ends_with(&format!(".{s}")));
        t.life != TabLife::Sleeping
            && !active
            && !t.audio.playing
            && !t.capture.camera
            && !t.capture.microphone
            && !t.capture.screen
            && !t.capture.location
            && !t.has_unsaved_input
            && !awake_site
            && t.agent.as_ref().is_none_or(|a| {
                !matches!(
                    a.status,
                    mado_types::state::AgentTabStatus::Working
                        | mado_types::state::AgentTabStatus::Waiting
                )
            })
    }

    fn energy_saver(&self) -> bool {
        match self.setting_str("performance.energySaver").as_str() {
            "battery20" => self.power.on_battery && self.power.percent <= 20,
            "unplugged" => self.power.on_battery,
            _ => false,
        }
    }

    /// The periodic check, or a memory pressure event.
    pub(super) fn sleep_check(&mut self, pressure: bool) {
        let now = self.now();
        let mode = self.setting_str("tabs.memorySaver");
        let after = match mode.as_str() {
            "moderate" | "balanced" => self
                .setting("tabs.sleepAfter")
                .as_i64()
                .unwrap_or(1_800_000),
            "maximum" => MAXIMUM_AFTER,
            _ => return,
        };
        let after = if self.energy_saver() {
            after / 2
        } else {
            after
        };
        let keep_awake = self.keep_awake_sites();
        let mut candidates: Vec<&TabState> = self
            .model
            .tabs
            .values()
            .filter(|t| self.can_sleep(t.id, &keep_awake))
            .collect();
        candidates.sort_by_key(|t| t.last_active_at);
        let chosen: Vec<TabId> = candidates
            .into_iter()
            .filter(|t| {
                let idle = now - t.last_active_at;
                idle >= after
                    || (pressure
                        && match mode.as_str() {
                            "moderate" => idle >= MODERATE_PRESSURE_IDLE,
                            _ => true,
                        })
            })
            .map(|t| t.id)
            .collect();
        for t in chosen {
            self.sleep_tab(t);
        }
    }

    /// Sleep procedure (02 §4.5): save the session and a snapshot, then the
    /// view goes (`Internal::SleepReady`).
    fn sleep_tab(&mut self, tab: TabId) {
        if !self.sleeping.insert(tab) {
            return;
        }
        let inputs = self.inputs.clone();
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::Snapshot {
                full_page: false,
                reply: Box::new({
                    let inputs = inputs.clone();
                    move |image: Result<Image, mado_engine::EngineError>| {
                        if let Ok(image) = image
                            && image.format == ImageFormat::Jpeg
                        {
                            let _ = inputs.send(super::Input::Internal(Internal::SnapshotTaken {
                                tab,
                                jpeg: image.bytes,
                            }));
                        }
                    }
                }),
            },
        ));
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::SaveSession {
                reply: Box::new(move |blob: Result<SessionBlob, mado_engine::EngineError>| {
                    let _ = inputs.send(super::Input::Internal(Internal::SleepReady {
                        tab,
                        blob: blob.ok(),
                    }));
                }),
            },
        ));
    }

    pub(super) fn sleep_ready(&mut self, tab: TabId, blob: Option<SessionBlob>) {
        self.sleeping.remove(&tab);
        let keep_awake = self.keep_awake_sites();
        if !self.can_sleep(tab, &keep_awake) {
            return;
        }
        if let Some(e) = self.model.extras.get_mut(&tab)
            && blob.is_some()
        {
            e.session = blob;
        }
        if let Some(t) = self.model.tabs.get_mut(&tab) {
            t.life = TabLife::Sleeping;
            t.progress = 0.0;
        }
    }

    /// Keeps the saved session fresh, so a crash or quit restores history.
    fn save_session(&mut self, tab: TabId) {
        let inputs = self.inputs.clone();
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::SaveSession {
                reply: Box::new(move |blob: Result<SessionBlob, mado_engine::EngineError>| {
                    if let Ok(blob) = blob {
                        let _ = inputs
                            .send(super::Input::Internal(Internal::SessionSaved { tab, blob }));
                    }
                }),
            },
        ));
    }

    // ----- startup (04 §8) -----

    pub(super) fn startup(
        &mut self,
        bounds: Bounds,
        rec: &mut PatchRecorder,
    ) -> Result<(), MadoError> {
        let loaded = std::mem::take(&mut self.loaded);
        let baseline = loaded.model.clone();
        let had_windows = !loaded.model.windows.is_empty();
        let mode = self.setting_str("startup.mode");
        let now = self.now();
        if mode == "continue" && had_windows {
            self.model = loaded.model;
            let active: Vec<TabId> = self
                .model
                .windows
                .values()
                .filter_map(|w| w.active_tab_id)
                .collect();
            for t in active {
                self.model.activate(t, now)?;
            }
            if self.state.profile.is_private {
                self.model = Model::default();
            }
        } else {
            self.model.closed_groups = loaded.model.closed_groups.clone();
            let pages: Vec<String> = if mode == "pages" {
                self.setting("startup.pages")
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let bounds = if bounds.width > 0.0 {
                bounds
            } else {
                loaded
                    .model
                    .windows
                    .values()
                    .next()
                    .map(|w| w.bounds.clone())
                    .unwrap_or_default()
            };
            let window = self.open_window(bounds, &pages)?;
            if had_windows && !self.state.profile.is_private {
                // Pinned tabs come back, sleeping.
                let mut pinned: Vec<TabSnapshot> = Vec::new();
                let mut windows = Vec::new();
                for w in loaded.model.windows.keys() {
                    if let Ok(s) = loaded.model.window_snapshot(*w) {
                        pinned.extend(
                            s.tabs
                                .iter()
                                .filter(|t| t.tab.pinned)
                                .map(|t| t.tab.clone()),
                        );
                        windows.push(s);
                    }
                }
                for (i, p) in pinned.iter().enumerate() {
                    self.model.restore_tab(window, i, None, p, now)?;
                }
                // The last session waits on the closed stack for ⌘⇧T.
                self.push_closed(ClosedEntry::Session { windows });
                if !self.clean_start {
                    let notice = Notice {
                        id: format!("n{}", self.next_notice),
                        message_key: "notice.restorePages".into(),
                        args: BTreeMap::new(),
                        action: Some(NoticeAction {
                            label_key: "notice.restore".into(),
                            command: Command::SessionReopenClosed,
                        }),
                        undo: None,
                        expires_at: None,
                    };
                    self.next_notice += 1;
                    self.add_notice(notice, rec);
                }
            }
        }
        // What is on disk is the baseline the next write compares with.
        self.persisted = baseline;
        self.timers.schedule(Timer::SleepCheck, SLEEP_CHECK);
        Ok(())
    }

    // ----- the document, the engine and the database -----

    /// After every handler: the document, the engine and the database
    /// follow the model. Returns engine calls to send before and after the
    /// handler's own (creations first, removals last).
    pub(super) fn reconcile(
        &mut self,
        before: &Model,
        rec: &mut PatchRecorder,
    ) -> (Vec<EngineOp>, Vec<EngineOp>) {
        {
            let mut tx = self.state.tx(rec);
            self.model.sync(before, &mut tx);
        }
        let after = &self.model;
        let mut first = Vec::new();
        let mut last = Vec::new();
        for (id, w) in &after.windows {
            if !before.windows.contains_key(id) {
                first.push(EngineOp::ApplyWindow(
                    *id,
                    WindowCommand::Create {
                        window: *id,
                        bounds: w.bounds.clone(),
                        kind: w.kind,
                        private: w.is_private,
                    },
                ));
            }
        }
        for id in before.windows.keys() {
            if !after.windows.contains_key(id) {
                last.push(EngineOp::ApplyWindow(*id, WindowCommand::Close));
            }
        }
        let live = |t: &TabState| t.life != TabLife::Sleeping;
        for (id, t) in &after.tabs {
            match before.tabs.get(id) {
                None if live(t) => {
                    if self.adopted.remove(id) {
                        // The engine made this view in the opener's window;
                        // put it where the core placed it.
                        first.push(EngineOp::Apply(*id, TabCommand::SetWindow(t.window_id)));
                    } else {
                        first.push(EngineOp::CreateTab(self.spec_for(t)));
                    }
                }
                None => {}
                Some(b) if !live(b) && live(t) => first.push(EngineOp::CreateTab(self.spec_for(t))),
                Some(b) if live(b) && !live(t) => last.push(EngineOp::DestroyTab(*id)),
                Some(b) if live(t) => {
                    if b.window_id != t.window_id {
                        first.push(EngineOp::Apply(*id, TabCommand::SetWindow(t.window_id)));
                    }
                    if b.audio.muted != t.audio.muted {
                        first.push(EngineOp::Apply(*id, TabCommand::SetMuted(t.audio.muted)));
                    }
                    if (b.zoom - t.zoom).abs() > f64::EPSILON {
                        first.push(EngineOp::Apply(*id, TabCommand::SetZoom(t.zoom)));
                    }
                }
                Some(_) => {}
            }
        }
        let gone: Vec<TabId> = before
            .tabs
            .keys()
            .filter(|id| !after.tabs.contains_key(id))
            .copied()
            .collect();
        for id in &gone {
            if before.tabs.get(id).is_some_and(live) {
                last.push(EngineOp::DestroyTab(*id));
            }
        }
        let profile = self.state.profile.name.clone();
        for (wid, w) in &after.windows {
            let was = before.windows.get(wid);
            let active = w.active_tab_id;
            let previous = was.and_then(|b| b.active_tab_id);
            let rect_changed = was.map(|b| b.content_rect) != Some(w.content_rect);
            if active != previous
                && let Some(p) = previous
                && after.tabs.get(&p).is_some_and(live)
            {
                first.push(EngineOp::Apply(p, TabCommand::SetVisible(false)));
            }
            if let Some(a) = active
                && (active != previous || rect_changed)
            {
                if let Some(rect) = w.content_rect {
                    first.push(EngineOp::Apply(a, TabCommand::SetFrame(rect)));
                }
                if active != previous {
                    first.push(EngineOp::Apply(a, TabCommand::SetVisible(true)));
                }
            }
            let title = |m: &Model, t: Option<TabId>| {
                t.and_then(|t| m.tabs.get(&t))
                    .map(|t| t.custom_title.clone().unwrap_or_else(|| t.title.clone()))
            };
            let now_title = title(after, active);
            if was.is_none() || now_title != title(before, previous) {
                let text = match (now_title.filter(|t| !t.is_empty()), profile.is_empty()) {
                    (Some(t), true) => t,
                    (Some(t), false) => format!("{t} – {profile}"),
                    (None, _) => "Mado".to_owned(),
                };
                first.push(EngineOp::ApplyWindow(*wid, WindowCommand::SetTitle(text)));
            }
        }
        for id in gone {
            self.forget_tab(id);
        }
        // A view made from a saved session reloads its page: history records
        // that visit as a restore (spec 12 §2.1).
        for op in &first {
            if let EngineOp::CreateTab(spec) = op
                && matches!(spec.initial, TabInitial::Restore(_))
            {
                self.pending_transition
                    .insert(spec.tab, crate::history::Transition::Restore);
            }
        }
        (first, last)
    }

    pub(super) fn spec_for(&self, t: &TabState) -> TabSpec {
        let window = self.model.windows.get(&t.window_id);
        let initial = match self.model.extras.get(&t.id).and_then(|e| e.session.clone()) {
            Some(blob) => TabInitial::Restore(blob),
            None => TabInitial::Url(t.url.clone()),
        };
        TabSpec {
            tab: t.id,
            window: t.window_id,
            initial,
            private: window.is_some_and(|w| w.is_private),
            agent: t.agent.is_some(),
            user_agent: self.user_agent.clone(),
            zoom: t.zoom,
            visible: window.is_some_and(|w| w.active_tab_id == Some(t.id)),
            frame: window.and_then(|w| w.content_rect),
            javascript: true,
        }
    }

    /// Writes the session rows that changed (never for private profiles).
    pub(super) fn persist_model(&mut self) {
        if self.state.profile.is_private || self.persisted == self.model {
            return;
        }
        let before = std::mem::replace(&mut self.persisted, self.model.clone());
        let after = self.model.clone();
        let now = self.now();
        let pending = self
            .profile_db
            .writer()
            .write(move |c| store::write(c, &before, &after, now));
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "the session was not written");
            }
        });
    }

    // ----- engine events about tabs -----

    pub(super) fn tab_event(&mut self, event: EngineEvent, rec: &mut PatchRecorder) {
        let now = self.now();
        match event {
            EngineEvent::WindowCreated { .. } | EngineEvent::TabCreated { .. } => {}
            EngineEvent::WindowCloseRequested { window } => {
                if self.model.windows.contains_key(&window) {
                    self.request_close(CloseAction::Window(window), rec);
                }
            }
            EngineEvent::NavigationStarted {
                tab,
                is_main_frame,
                kind,
                user_gesture,
                ..
            } => {
                if is_main_frame {
                    self.nav_started(tab, kind, user_gesture);
                }
                if is_main_frame && let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.life = TabLife::Loading;
                    t.progress = 0.1;
                }
            }
            EngineEvent::NavigationCommitted {
                tab,
                url,
                is_same_document,
                ..
            } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.security.level = security_of(&url, None);
                    t.url = url;
                    if !is_same_document {
                        t.has_unsaved_input = false;
                    }
                }
                if !is_same_document {
                    let url = self
                        .model
                        .tabs
                        .get(&tab)
                        .map(|t| t.url.clone())
                        .unwrap_or_default();
                    self.nav_committed(tab, &url);
                }
                if !is_same_document {
                    self.new_document(tab, rec);
                }
                self.save_session(tab);
            }
            EngineEvent::NavigationFinished { tab, .. }
            | EngineEvent::NavigationFailed { tab, .. } => {
                if let Some(t) = self.model.tabs.get_mut(&tab)
                    && t.life == TabLife::Loading
                {
                    t.life = TabLife::Live;
                    t.progress = 1.0;
                }
            }
            EngineEvent::TitleChanged { tab, title } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.title = title.chars().take(1024).collect();
                }
                self.title_changed(tab);
            }
            EngineEvent::UrlChanged { tab, url } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.url = url;
                }
            }
            EngineEvent::ProgressChanged { tab, progress } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.progress = progress.clamp(0.0, 1.0);
                }
            }
            EngineEvent::HistoryStateChanged {
                tab,
                can_go_back,
                can_go_forward,
            } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.can_go_back = can_go_back;
                    t.can_go_forward = can_go_forward;
                }
            }
            EngineEvent::SecurityChanged { tab, state } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.security.level = security_of(&t.url, Some(&state));
                }
            }
            EngineEvent::AudioChanged { tab, playing } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.audio.playing = playing;
                }
            }
            EngineEvent::CaptureChanged {
                tab,
                camera,
                microphone,
                screen,
            } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.capture.camera = camera;
                    t.capture.microphone = microphone;
                    t.capture.screen = screen;
                }
            }
            EngineEvent::HoverLink { tab, url } => {
                if let Some(t) = self.model.tabs.get(&tab)
                    && let Some(w) = self.model.windows.get_mut(&t.window_id)
                    && w.active_tab_id == Some(tab)
                {
                    w.hover_link = url;
                }
            }
            EngineEvent::FindResult {
                tab,
                matches,
                active,
            } => {
                if let Some(t) = self.model.tabs.get(&tab)
                    && let Some(w) = self.model.windows.get_mut(&t.window_id)
                    && w.active_tab_id == Some(tab)
                    && let Some(find) = w.find_bar.as_mut()
                {
                    find.matches = matches;
                    find.active = active;
                }
            }
            EngineEvent::ProcessGone { tab, reason } => {
                if let Some(t) = self.model.tabs.get_mut(&tab) {
                    t.life = TabLife::Crashed;
                    t.progress = 0.0;
                    t.audio.playing = false;
                    t.capture = mado_types::state::CaptureState::default();
                    tracing::warn!(%tab, killed = reason == GoneReason::Killed, "a tab's content process went away");
                }
            }
            EngineEvent::CloseRequested { tab } => {
                if self.model.tabs.contains_key(&tab) {
                    self.close_tabs(&[tab], false);
                }
            }
            EngineEvent::PageMessage {
                tab,
                frame,
                message,
                ..
            } => self.page_message(tab, &frame, message, rec),
            EngineEvent::DownloadStarted {
                tab,
                download,
                reply,
            } => self.download_started(tab, download, reply, rec),
            EngineEvent::DownloadProgress {
                id,
                received,
                total,
            } => self.download_progress(id, received, total, rec),
            EngineEvent::DownloadFinished { id, result } => self.download_finished(id, result, rec),
            EngineEvent::NewWindowRequested {
                tab,
                request,
                reply,
            } => {
                let decision = self.popup(tab, &request, now);
                reply.send(decision);
            }
            other => tracing::trace!(event = other.name(), "engine event not handled here"),
        }
    }

    /// Where a page's new window goes (spec 05 §7). Pop-ups without a user
    /// gesture are blocked; the per-site allow list and the address bar's
    /// blocked list come with the address bar (M2-T06).
    fn popup(&mut self, opener: TabId, request: &PopupRequest, now: i64) -> PopupDecision {
        let Some(window) = self.model.tabs.get(&opener).map(|t| t.window_id) else {
            return PopupDecision::Block;
        };
        if !request.user_gesture {
            return PopupDecision::Block;
        }
        let m = request.modifiers;
        let decision = if request.features.wants_window() || (m.shift && !m.command) {
            PopupDecision::Window
        } else {
            PopupDecision::Tab {
                foreground: !(m.command || m.button == 1) || m.shift,
            }
        };
        let prefs = self.prefs();
        let placed = match decision {
            PopupDecision::Window => {
                let id = WindowId::new();
                let f = &request.features;
                let bounds = Bounds {
                    x: f.x.unwrap_or(0.0),
                    y: f.y.unwrap_or(0.0),
                    width: f.width.unwrap_or(0.0),
                    height: f.height.unwrap_or(0.0),
                    screen: String::new(),
                };
                let private = self.state.profile.is_private;
                let kind = if request.features.wants_window() {
                    WindowKind::Popup
                } else {
                    WindowKind::Normal
                };
                self.model
                    .add_window_of(id, kind, bounds, self.layout(), private);
                self.model.adopt_tab(
                    request.tab,
                    id,
                    request.url.clone(),
                    None,
                    true,
                    Some(opener),
                    now,
                    prefs,
                )
            }
            PopupDecision::Tab { foreground } => self.model.adopt_tab(
                request.tab,
                window,
                request.url.clone(),
                None,
                foreground,
                Some(opener),
                now,
                prefs,
            ),
            PopupDecision::Block => return PopupDecision::Block,
        };
        match placed {
            Ok(_) => {
                self.adopted.insert(request.tab);
                decision
            }
            Err(_) => PopupDecision::Block,
        }
    }
}

fn snapshot(tab: &TabState, extra: &crate::tabs::TabExtra) -> TabSnapshot {
    TabSnapshot {
        url: tab.url.clone(),
        title: tab.title.clone(),
        favicon: tab.favicon.clone(),
        pinned: tab.pinned,
        custom_title: tab.custom_title.clone(),
        session_blob: extra
            .session
            .as_ref()
            .map(|b| base64::engine::general_purpose::STANDARD.encode(&b.0)),
        scroll: extra.scroll,
        pinned_home_url: extra.pinned_home_url.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_with_userinfo_warns() {
        assert_eq!(
            security_of("https://user:pw@bank.example/", None),
            SecurityLevel::NotSecure
        );
        assert_eq!(
            security_of("https://bank.example/", None),
            SecurityLevel::Secure
        );
    }
}
