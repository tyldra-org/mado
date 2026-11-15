//! Site permissions in the core (spec 07 §2 to §5): the decision flow,
//! prompts, OS checks, the notification and geolocation shims, capture
//! indicators and auto-revoke.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mado_engine::event::{PermissionDecision, PermissionRequest};
use mado_engine::{EngineOp, PermissionKind, Reply, TabCommand};
use mado_permissions::{Row, Source, Value as Site};
use mado_platform::permissions::{OsPermission, OsStatus};
use mado_types::command::{CaptureKind, Command, Origin, PromptDecision, SiteValue};
use mado_types::state::{Notice, NoticeAction, PromptState};
use mado_types::{ErrorCode, MadoError, PatchRecorder, PromptId, TabId};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::{Core, Internal, Reply as CmdReply};

/// Buttons ignore clicks this long after the prompt appears (07 §3).
pub const ARM_MS: i64 = 500;

/// A prompt waiting for an answer.
pub struct Pending {
    tab: TabId,
    origin: String,
    key: String,
    reply: Reply<PermissionDecision>,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("tab", &self.tab)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

/// Requests queued behind a tab's open prompt.
pub type Queue = BTreeMap<TabId, VecDeque<(PermissionRequest, Reply<PermissionDecision>)>>;

/// The key's name in `site_settings` (07 §1).
pub fn key_name(kind: &PermissionKind) -> String {
    match kind {
        PermissionKind::ExternalApp(scheme) => format!("externalApp:{scheme}"),
        other => serde_json::to_value(other)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
    }
}

fn os_kind(key: &str) -> Option<OsPermission> {
    Some(match key {
        "camera" => OsPermission::Camera,
        "microphone" => OsPermission::Microphone,
        "location" => OsPermission::Location,
        "screen" => OsPermission::Screen,
        "speechRecognition" => OsPermission::Speech,
        _ => return None,
    })
}

/// Every `site_settings` row.
pub fn load(c: &Connection) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = c.prepare(
        "SELECT origin, key, value, embedder, last_used_at, granted_at, expires_at, source FROM site_settings",
    )?;
    let rows = stmt.query_map([], |r| {
        let value: String = r.get(2)?;
        let source: String = r.get(7)?;
        Ok((
            Row {
                origin: r.get(0)?,
                key: r.get(1)?,
                value: Site::parse(&value).unwrap_or(Site::Ask),
                embedder: r.get(3)?,
                last_used_at: r.get(4)?,
                granted_at: r.get(5)?,
                expires_at: r.get(6)?,
                source: Source::parse(&source).unwrap_or(Source::User),
            },
            value,
        ))
    })?;
    // Structured values (httpAllowed, lookalikeIgnored) belong to their
    // owners, not to the permission store.
    Ok(rows
        .filter_map(Result::ok)
        .filter(|(_, v)| Site::parse(v).is_some())
        .map(|(r, _)| r)
        .collect())
}

fn site_value(v: SiteValue) -> Site {
    match v {
        SiteValue::Allow => Site::Allow,
        SiteValue::Block => Site::Block,
        SiteValue::Ask => Site::Ask,
    }
}

impl Core {
    /// The key's default: the `permissions.<key>` setting, else 07 §1.
    fn key_default(&self, key: &str) -> Site {
        let setting = format!("permissions.{key}");
        if let Some(def) = mado_settings::lookup(&setting) {
            return self
                .settings
                .value(def)
                .as_str()
                .and_then(Site::parse)
                .unwrap_or(Site::Ask);
        }
        match key {
            "motionSensors" | "sound" | "javascript" | "images" | "protectedContent" => Site::Allow,
            "popups" | "insecureContent" | "automaticFullscreen" | "applePay" => Site::Block,
            _ => Site::Ask,
        }
    }

    /// The site's value for `key` on `origin`, no prompt.
    pub(super) fn site_value(&self, origin: &str, key: &str, embedder: Option<&str>) -> Site {
        let now = self.now();
        self.sites
            .decide(origin, key, embedder, self.key_default(key), now)
    }

    fn persist_site(&self, row: Option<Row>, removed: Vec<Row>) {
        if self.state.profile.is_private {
            return;
        }
        let pending = self.profile_db.writer().write(move |c| {
            for r in &removed {
                c.execute(
                    "DELETE FROM site_settings WHERE origin = ?1 AND key = ?2 AND embedder = ?3",
                    params![r.origin, r.key, r.embedder],
                )?;
            }
            if let Some(r) = row {
                c.execute(
                    "INSERT OR REPLACE INTO site_settings (origin, key, value, embedder, last_used_at, granted_at, expires_at, source)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        r.origin,
                        r.key,
                        r.value.as_str(),
                        r.embedder,
                        r.last_used_at,
                        r.granted_at,
                        r.expires_at,
                        r.source.as_str()
                    ],
                )?;
            }
            Ok(())
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "a site setting was not saved");
            }
        });
    }

    fn write_site(
        &mut self,
        origin: &str,
        key: &str,
        embedder: Option<&str>,
        value: Site,
        source: Source,
    ) {
        let row = Row {
            origin: origin.to_owned(),
            key: key.to_owned(),
            value,
            embedder: embedder.unwrap_or_default().to_owned(),
            last_used_at: None,
            granted_at: self.now(),
            expires_at: None,
            source,
        };
        self.sites.set(row.clone());
        self.persist_site(Some(row), Vec::new());
    }

    /// 07 §2: grant, deny, or ask.
    pub(super) fn permission_requested(
        &mut self,
        tab: TabId,
        request: PermissionRequest,
        reply: Reply<PermissionDecision>,
        rec: &mut PatchRecorder,
    ) {
        let key = key_name(&request.kind);
        eprintln!(
            "DBG perm key={key} value={:?}",
            self.site_value(&request.origin, &key, request.embedder.as_deref())
        );
        let agent = self.model.tabs.get(&tab).is_some_and(|t| t.agent.is_some());
        if agent
            && matches!(
                key.as_str(),
                "camera" | "microphone" | "screen" | "location"
            )
        {
            reply.send(PermissionDecision::Deny);
            return;
        }
        let embedder = request.embedder.as_deref();
        match self.site_value(&request.origin, &key, embedder) {
            Site::Allow => {
                let now = self.now();
                let touched = self
                    .sites
                    .touch(&request.origin, &key, embedder.unwrap_or_default(), now)
                    .or_else(|| self.sites.touch(&request.origin, &key, "", now));
                if touched.is_some() {
                    self.persist_site(touched, Vec::new());
                }
                reply.send(PermissionDecision::Grant);
                return;
            }
            Site::Block => {
                reply.send(PermissionDecision::Deny);
                return;
            }
            Site::Ask => {}
        }
        if self
            .this_time
            .get(&tab)
            .is_some_and(|s| s.contains(&(request.origin.clone(), key.clone())))
        {
            reply.send(PermissionDecision::Grant);
            return;
        }
        if let Some(os) = os_kind(&key)
            && matches!(
                (self.os.status)(os),
                OsStatus::Denied | OsStatus::Restricted
            )
        {
            reply.send(PermissionDecision::Deny);
            self.os_denied_notice(&key, rec);
            return;
        }
        // Quiet mode: three dismissals and the site stops asking.
        let now = self.now();
        if key == "notifications" && self.dismissals.count(&request.origin, &key, now) >= 3 {
            reply.send(PermissionDecision::Deny);
            return;
        }
        if self.pending_perms.values().any(|p| p.tab == tab) {
            self.perm_queue
                .entry(tab)
                .or_default()
                .push_back((request, reply));
            return;
        }
        self.show_permission_prompt(tab, request, key, reply, rec);
    }

    fn show_permission_prompt(
        &mut self,
        tab: TabId,
        request: PermissionRequest,
        key: String,
        reply: Reply<PermissionDecision>,
        rec: &mut PatchRecorder,
    ) {
        let id = PromptId::new();
        let now = self.now();
        let mut prompts = self.state.prompts.clone();
        prompts.insert(
            id,
            PromptState::Permission {
                id,
                tab_id: tab,
                origin: request.origin.clone(),
                embedder: request.embedder.clone(),
                keys: vec![key.clone()],
                created_at: now,
                armed_at: now + ARM_MS,
            },
        );
        self.state.tx(rec).set_prompts(prompts);
        self.pending_perms.insert(
            id,
            Pending {
                tab,
                origin: request.origin,
                key,
                reply,
            },
        );
    }

    fn os_denied_notice(&mut self, key: &str, rec: &mut PatchRecorder) {
        let id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let notice = Notice {
            id,
            message_key: "notice.osPermissionDenied".into(),
            args: BTreeMap::from([("permission".to_owned(), json!(key))]),
            action: Some(NoticeAction {
                label_key: "notice.openSystemSettings".into(),
                command: Command::AppOpenSystemSettings {
                    pane: key.to_owned(),
                },
            }),
            undo: None,
            expires_at: Some(self.now() + 15_000),
        };
        self.add_notice(notice, rec);
    }

    /// Takes the tab's prompt off screen and shows its next queued one.
    fn close_prompt(&mut self, id: PromptId, rec: &mut PatchRecorder) -> Option<Pending> {
        let pending = self.pending_perms.remove(&id)?;
        let mut prompts = self.state.prompts.clone();
        prompts.remove(&id);
        self.state.tx(rec).set_prompts(prompts);
        let tab = pending.tab;
        if let Some((request, reply)) = self.perm_queue.get_mut(&tab).and_then(VecDeque::pop_front)
        {
            self.permission_requested(tab, request, reply, rec);
        }
        Some(pending)
    }

    pub(super) fn prompt_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
        reply_to: &mut Option<CmdReply>,
    ) -> Result<Value, MadoError> {
        let not_found = || MadoError::new(ErrorCode::NotFound, "error.prompt.notFound");
        // Prompts are answered in Mado's own UI; site settings change
        // there too, never from a control client or an agent (07 §3).
        let mado_ui = origin.is_user();
        match cmd {
            Command::PromptRespond { id, decision } => {
                let armed = match self.state.prompts.get(&id) {
                    Some(PromptState::Permission { armed_at, .. }) => *armed_at,
                    _ => return Err(not_found()),
                };
                if !mado_ui {
                    return Err(MadoError::new(
                        ErrorCode::PermissionDenied,
                        "error.prompt.notAllowed",
                    ));
                }
                if self.now() < armed {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.prompt.notArmed",
                    ));
                }
                let p = self.close_prompt(id, rec).ok_or_else(not_found)?;
                match decision {
                    PromptDecision::Never => {
                        self.write_site(&p.origin, &p.key, None, Site::Block, Source::User);
                        p.reply.send(PermissionDecision::Deny);
                    }
                    PromptDecision::AllowWhileVisiting | PromptDecision::AllowThisTime => {
                        if decision == PromptDecision::AllowWhileVisiting {
                            self.write_site(&p.origin, &p.key, None, Site::Allow, Source::User);
                        } else {
                            self.this_time
                                .entry(p.tab)
                                .or_default()
                                .insert((p.origin.clone(), p.key.clone()));
                        }
                        self.grant_after_os(p);
                    }
                }
                Ok(Value::Null)
            }
            Command::PromptDismiss { id } => {
                if !matches!(
                    self.state.prompts.get(&id),
                    Some(PromptState::Permission { .. })
                ) {
                    return Err(not_found());
                }
                if let Some(p) = self.close_prompt(id, rec) {
                    self.dismissed(p);
                }
                Ok(Value::Null)
            }
            Command::PermissionList { origin: only } => {
                let rows: Vec<&Row> = self
                    .sites
                    .rows()
                    .filter(|r| only.as_deref().is_none_or(|o| o == r.origin))
                    .collect();
                if let Some(r) = reply_to.take() {
                    let _ = r.send(Ok(json!(rows)));
                }
                Ok(Value::Null)
            }
            Command::PermissionSet {
                origin: site,
                key,
                value,
                embedder,
            } => {
                if !mado_ui {
                    return Err(MadoError::new(
                        ErrorCode::PermissionDenied,
                        "error.permission.notAllowed",
                    ));
                }
                if mado_permissions::origin_of(&site).as_deref() != Some(site.as_str())
                    && !site.starts_with("[*.]")
                {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.permission.badOrigin",
                    ));
                }
                self.write_site(
                    &site,
                    &key,
                    embedder.as_deref(),
                    site_value(value),
                    Source::User,
                );
                Ok(Value::Null)
            }
            Command::PermissionReset { origin: site, key } => {
                if !mado_ui {
                    return Err(MadoError::new(
                        ErrorCode::PermissionDenied,
                        "error.permission.notAllowed",
                    ));
                }
                let removed = self.sites.remove(&site, key.as_deref());
                self.persist_site(None, removed);
                Ok(Value::Null)
            }
            Command::SiteStopCapture { tab_id, kind } => {
                self.model
                    .tabs
                    .get(&tab_id)
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.tab.notFound"))?;
                let kind = match kind {
                    CaptureKind::Camera => PermissionKind::Camera,
                    CaptureKind::Microphone => PermissionKind::Microphone,
                    CaptureKind::Screen => PermissionKind::Screen,
                    CaptureKind::Location => {
                        self.post(tab_id, json!({ "type": "geolocationStop" }));
                        if let Some(t) = self.model.tabs.get_mut(&tab_id) {
                            t.capture.location = false;
                        }
                        return Ok(Value::Null);
                    }
                };
                self.outbox
                    .push(EngineOp::Apply(tab_id, TabCommand::StopCapture(kind)));
                Ok(Value::Null)
            }
            other => {
                tracing::error!(command = other.name(), "a command reached no handler");
                Err(MadoError::internal())
            }
        }
    }

    /// Dismissing denies this request; the third time in a week blocks.
    fn dismissed(&mut self, p: Pending) {
        let now = self.now();
        if self.dismissals.record(&p.origin, &p.key, now) {
            self.write_site(&p.origin, &p.key, None, Site::Block, Source::Default);
        }
        p.reply.send(PermissionDecision::Deny);
    }

    /// The OS prompt comes only after the user allowed the site (07 §4).
    fn grant_after_os(&mut self, p: Pending) {
        let Some(os) = os_kind(&p.key) else {
            p.reply.send(PermissionDecision::Grant);
            return;
        };
        if (self.os.status)(os) != OsStatus::NotDetermined {
            p.reply.send(PermissionDecision::Grant);
            return;
        }
        let inputs = self.inputs.clone();
        let request = self.os.request;
        let token = self.next_os_wait;
        self.next_os_wait += 1;
        self.os_waiting.insert(token, p.reply);
        tokio::task::spawn_blocking(move || {
            let granted = request(os) == OsStatus::Granted;
            let _ = inputs.send(super::Input::Internal(Internal::OsAnswered {
                token,
                granted,
            }));
        });
    }

    pub(super) fn os_answered(&mut self, token: u64, granted: bool) {
        if let Some(reply) = self.os_waiting.remove(&token) {
            reply.send(if granted {
                PermissionDecision::Grant
            } else {
                PermissionDecision::Deny
            });
        }
    }

    /// A new document or a closed tab: its prompts deny, its queue empties
    /// and "this time" grants end.
    pub(super) fn forget_permissions(&mut self, tab: TabId, rec: &mut PatchRecorder) {
        self.this_time.remove(&tab);
        if let Some(queue) = self.perm_queue.remove(&tab) {
            for (_, reply) in queue {
                reply.send(PermissionDecision::Deny);
            }
        }
        let ids: Vec<PromptId> = self
            .pending_perms
            .iter()
            .filter(|(_, p)| p.tab == tab)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(p) = self.close_prompt(id, rec) {
                p.reply.send(PermissionDecision::Deny);
            }
        }
    }

    /// The notifications shim asks (07 §1.1): the flow decides, and the
    /// page hears granted, denied or default.
    pub(super) fn shim_permission(
        &mut self,
        tab: TabId,
        kind: PermissionKind,
        then: ShimThen,
        rec: &mut PatchRecorder,
    ) {
        let Some(origin) = self
            .model
            .tabs
            .get(&tab)
            .and_then(|t| mado_permissions::origin_of(&t.url))
        else {
            return;
        };
        let inputs = self.inputs.clone();
        let (reply, _expiry) =
            Reply::new(PermissionDecision::Deny, move |d: PermissionDecision| {
                let _ = inputs.send(super::Input::Internal(Internal::ShimDecided {
                    tab,
                    then,
                    granted: d == PermissionDecision::Grant,
                }));
            });
        self.permission_requested(
            tab,
            PermissionRequest {
                kind,
                origin,
                embedder: None,
            },
            reply,
            rec,
        );
    }

    pub(super) fn shim_decided(&mut self, tab: TabId, then: ShimThen, granted: bool) {
        match then {
            ShimThen::Notifications => {
                let permission = if granted { "granted" } else { "denied" };
                self.post(
                    tab,
                    json!({ "type": "notificationPermission", "permission": permission }),
                );
            }
            ShimThen::Geolocation(id) => {
                // Granted or not, Mado has no location provider before
                // WebKit's own (macOS 27): position unavailable (2) or
                // permission denied (1).
                let error = if granted { 2 } else { 1 };
                self.post(
                    tab,
                    json!({ "type": "geolocationPosition", "id": id, "error": error }),
                );
            }
        }
    }

    /// After a commit: the shim learns the page's notification permission,
    /// JavaScript and sound follow the site.
    pub(super) fn site_committed(&mut self, tab: TabId, url: &str) {
        let Some(origin) = mado_permissions::origin_of(url) else {
            return;
        };
        let notifications = match self.site_value(&origin, "notifications", None) {
            Site::Allow => "granted",
            Site::Block => "denied",
            Site::Ask => "default",
        };
        self.post(
            tab,
            json!({ "type": "notificationPermission", "permission": notifications }),
        );
        let muted = self.site_value(&origin, "sound", None) == Site::Block;
        if muted && self.model.tabs.get(&tab).is_some_and(|t| !t.audio.muted) {
            if let Some(t) = self.model.tabs.get_mut(&tab) {
                t.audio.muted = true;
            }
            self.outbox
                .push(EngineOp::Apply(tab, TabCommand::SetMuted(true)));
        }
    }

    /// A page notification, shown by macOS when the site may notify.
    pub(super) fn show_notification(
        &mut self,
        tab: TabId,
        n: &mado_engine::page::NotificationShow,
    ) {
        let Some(origin) = self
            .model
            .tabs
            .get(&tab)
            .and_then(|t| mado_permissions::origin_of(&t.url))
        else {
            return;
        };
        if self.site_value(&origin, "notifications", None) != Site::Allow {
            return;
        }
        let id = format!("{tab}:{}", n.id);
        let host = url::Url::parse(&origin)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        self.ui
            .show_notification(&id, &n.title, &format!("{host}: {}", n.body));
    }

    /// macOS reports a click on a page notification: activate its tab and
    /// tell the page.
    pub(super) fn notification_clicked(&mut self, id: &str) {
        let Some((tab, n)) = id.split_once(':') else {
            return;
        };
        let (Ok(tab), Ok(n)) = (tab.parse::<TabId>(), n.parse::<u32>()) else {
            return;
        };
        if !self.model.tabs.contains_key(&tab) {
            return;
        }
        let now = self.now();
        let _ = self.model.activate(tab, now);
        self.post(tab, json!({ "type": "notificationClick", "id": n }));
    }

    /// 07 §5: daily, unused grants go back to `ask`, with Undo.
    pub(super) fn auto_revoke(&mut self, rec: &mut PatchRecorder) {
        let now = self.now();
        let readers = self.profile_db.readers();
        let visited = |origin: &str| {
            let host = url::Url::parse(origin)
                .ok()
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default();
            readers
                .read(|c| {
                    c.query_row(
                        "SELECT EXISTS (SELECT 1 FROM visits v JOIN urls u ON u.id = v.url_id WHERE u.host = ?1 AND v.visited_at > ?2)",
                        params![host, now - 60 * 86_400_000],
                        |r| r.get::<_, bool>(0),
                    )
                })
                .unwrap_or(true)
        };
        let revoked = self.sites.auto_revoke(now, visited);
        if revoked.is_empty() {
            return;
        }
        for r in &revoked {
            self.persist_site(Some(r.clone()), Vec::new());
        }
        let sites: BTreeSet<String> = revoked.iter().map(|r| r.origin.clone()).collect();
        let token = self.next_notice;
        self.revoked.insert(token, revoked);
        let id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let undo = Command::PermissionUndoRevoke { token };
        let notice = Notice {
            id,
            message_key: "notice.permissionsRevoked".into(),
            args: BTreeMap::from([(
                "sites".to_owned(),
                json!(sites.into_iter().collect::<Vec<_>>().join(", ")),
            )]),
            action: Some(NoticeAction {
                label_key: "notice.undo".into(),
                command: undo.clone(),
            }),
            undo: Some(undo),
            expires_at: Some(now + 30_000),
        };
        self.add_notice(notice, rec);
    }

    pub(super) fn undo_revoke(&mut self, token: u64) -> Result<Value, MadoError> {
        let rows = self
            .revoked
            .remove(&token)
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.permission.undoExpired"))?;
        for mut r in rows {
            r.source = Source::User;
            self.sites.set(r.clone());
            self.persist_site(Some(r), Vec::new());
        }
        Ok(Value::Null)
    }
}

/// What a shim's permission answer goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShimThen {
    Notifications,
    Geolocation(u32),
}
