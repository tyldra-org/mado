//! Cosmetic filtering, scriptlets and the element picker (spec 10 §6,
//! §8): the blocker's data for each main-frame navigation, generic hiding
//! for the class and id names a page reports, and hidden elements.

use std::collections::HashSet;

use mado_engine::page::{CosmeticClasses, PickerResult};
use mado_engine::{EngineOp, InjectAt, NavRequest, PageScript, TabCommand, World};
use mado_types::command::Command;
use mado_types::patch::PatchRecorder;
use mado_types::state::{Notice, NoticeAction};
use mado_types::{ErrorCode, MadoError, TabId};
use rusqlite::params;
use serde_json::{Value, json};

use super::{Core, Internal};

impl Core {
    /// Whether the blocker applies to `url` in this profile.
    fn blocker_applies(&self, url: &str) -> Option<String> {
        if self.blocker.is_none() || self.setting("blocker.enabled") != true {
            return None;
        }
        let u = url::Url::parse(url).ok()?;
        if !matches!(u.scheme(), "http" | "https") {
            return None;
        }
        let host = u.host_str()?.to_ascii_lowercase();
        (!mado_engine::host_in(&host, &self.blocker_off_hosts)).then_some(host)
    }

    /// A main-frame navigation is about to be allowed (10 §6 steps 1 and
    /// 2): its scripts go to the engine before the decision does.
    pub(super) fn cosmetic_navigation(&mut self, tab: TabId, request: &NavRequest) {
        let mut scripts = Vec::new();
        let mut exceptions = HashSet::new();
        if self.blocker_applies(&request.url).is_some() {
            let mut hide = Vec::new();
            let mut procedural = Vec::new();
            let mut injected = String::new();
            let mut generic = true;
            for engine in [&self.blocker_engine, &self.blocker_custom]
                .into_iter()
                .flatten()
            {
                let r = engine.url_cosmetic_resources(&request.url);
                hide.extend(r.hide_selectors);
                procedural.extend(r.procedural_actions);
                exceptions.extend(r.exceptions);
                if !r.injected_script.is_empty() {
                    injected.push_str(&r.injected_script);
                    injected.push('\n');
                }
                generic &= !r.generichide;
            }
            hide.sort();
            procedural.sort();
            let data = json!({ "hide": hide, "procedural": procedural, "generic": generic });
            scripts.push(PageScript {
                name: "cosmetic-data",
                world: World::Mado,
                at: InjectAt::DocumentStart,
                main_frame_only: false,
                source: format!("globalThis[Symbol.for(\"mado.cosmetic\")] = {data};"),
            });
            if !injected.is_empty() {
                scripts.push(PageScript {
                    name: "scriptlets",
                    world: World::Page,
                    at: InjectAt::DocumentStart,
                    main_frame_only: false,
                    source: format!("try {{\n{injected}\n}} catch (e) {{}}"),
                });
            }
        }
        let had = self.cosmetic_tabs.contains_key(&tab);
        if scripts.is_empty() {
            self.cosmetic_tabs.remove(&tab);
            if !had {
                return;
            }
        } else {
            self.cosmetic_tabs.insert(tab, exceptions);
        }
        // Straight to the engine, ahead of the navigation's answer.
        self.engine.send(EngineOp::Apply(
            tab,
            TabCommand::SetNavigationScripts(scripts),
        ));
    }

    /// 10 §6 step 3: generic rules for the names the page reported.
    pub(super) fn cosmetic_classes(&mut self, tab: TabId, msg: CosmeticClasses) {
        let Some(exceptions) = self.cosmetic_tabs.get(&tab) else {
            return;
        };
        let classes: Vec<&String> = msg.classes.iter().take(1000).collect();
        let ids: Vec<&String> = msg.ids.iter().take(1000).collect();
        let mut selectors = Vec::new();
        for engine in [&self.blocker_engine, &self.blocker_custom]
            .into_iter()
            .flatten()
        {
            selectors.extend(engine.hidden_class_id_selectors(&classes, &ids, exceptions));
        }
        if !selectors.is_empty() {
            self.post(
                tab,
                json!({ "type": "cosmeticSelectors", "selectors": selectors }),
            );
        }
    }

    fn page_host(&self, tab: TabId) -> Option<String> {
        let url = self.model.tabs.get(&tab)?.url.clone();
        url::Url::parse(&url)
            .ok()?
            .host_str()
            .map(str::to_ascii_lowercase)
    }

    /// The picker chose an element (10 §8): it is hidden on the host from
    /// now on, with Undo.
    pub(super) fn picker_result(&mut self, tab: TabId, msg: PickerResult, rec: &mut PatchRecorder) {
        let Some(selector) = msg.selector.filter(|s| !s.is_empty() && s.len() <= 1024) else {
            return;
        };
        let Some(host) = self.page_host(tab) else {
            return;
        };
        if self.state.profile.is_private {
            return;
        }
        let id = self.now();
        let row = (id, host.clone(), selector);
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "INSERT INTO blocker_hidden (id, host, selector, created_at) VALUES (?1, ?2, ?3, ?1)",
                params![row.0, row.1, row.2],
            )
            .map(|_| ())
        });
        self.after_blocker_write(pending);
        let notice_id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let notice = Notice {
            id: notice_id,
            message_key: "notice.elementHidden".into(),
            args: std::collections::BTreeMap::from([("host".to_owned(), json!(host))]),
            action: None::<NoticeAction>,
            undo: Some(Command::BlockerUnhide { id }),
            expires_at: Some(self.now() + 10_000),
        };
        self.add_notice(notice, rec);
    }

    /// Reloads the profile's filters once a write lands.
    fn after_blocker_write(&self, pending: mado_storage::PendingWrite<()>) {
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            match pending.await {
                Ok(()) => {
                    let _ = inputs.send(super::Input::Internal(Internal::BlockerProfileChanged));
                }
                Err(e) => tracing::error!(error = %e, "a blocker change was not saved"),
            }
        });
    }

    /// The blocker commands of 10 §8 that change pages and filters.
    pub(super) fn blocker_command(&mut self, cmd: Command) -> Result<Value, MadoError> {
        match cmd {
            Command::BlockerPickElement { tab_id } => {
                if !self.cosmetic_tabs.contains_key(&tab_id) {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.blocker.notOnPage",
                    ));
                }
                self.post(tab_id, json!({ "type": "pickerStart" }));
                Ok(Value::Null)
            }
            Command::BlockerHidden { host } => {
                let host = host.to_ascii_lowercase();
                let rows = self
                    .profile_db
                    .readers()
                    .read(move |c| {
                        c.prepare("SELECT id, selector, created_at FROM blocker_hidden WHERE host = ?1 ORDER BY id")?
                            .query_map([host], |r| {
                                Ok(json!({ "id": r.get::<_, i64>(0)?, "selector": r.get::<_, String>(1)?, "createdAt": r.get::<_, i64>(2)? }))
                            })?
                            .collect::<rusqlite::Result<Vec<Value>>>()
                    })
                    .map_err(|_| MadoError::internal())?;
                Ok(json!(rows))
            }
            Command::BlockerUnhide { id } => {
                let pending = self.profile_db.writer().write(move |c| {
                    c.execute("DELETE FROM blocker_hidden WHERE id = ?1", [id])
                        .map(|_| ())
                });
                self.after_blocker_write(pending);
                Ok(Value::Null)
            }
            Command::BlockerSetCustomFilters { text } => {
                if text.len() > 1024 * 1024 {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.blocker.tooLong",
                    ));
                }
                let lines: Vec<String> = text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect();
                let now = self.now();
                let pending = self.profile_db.writer().write(move |c| {
                    c.execute("DELETE FROM blocker_custom_filters", [])?;
                    for line in &lines {
                        c.execute(
                            "INSERT INTO blocker_custom_filters (line, created_at) VALUES (?1, ?2)",
                            params![line, now],
                        )?;
                    }
                    Ok(())
                });
                self.after_blocker_write(pending);
                Ok(Value::Null)
            }
            _ => Err(MadoError::new(
                ErrorCode::InvalidInput,
                "error.command.unknown",
            )),
        }
    }
}
