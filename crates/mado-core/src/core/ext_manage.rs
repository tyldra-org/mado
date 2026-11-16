//! Extension management (spec 09 §6, §11): enabling, removal, site
//! access, optional permissions, shortcuts, pins, reload, Developer mode
//! errors and inspection, the toolbar and its popups.

use mado_engine::{EngineOp, ExtensionOp};
use mado_types::command::{Command, ExtensionSource, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::state::{ExtensionButton, InstalledExtension, SiteAccess, SiteAccessMode};
use mado_types::{ErrorCode, ExtensionId, MadoError};
use rusqlite::params;
use serde_json::{Value, json};

use super::{Core, Reply};

/// Whether match pattern `p` covers `url` (`<all_urls>`,
/// `scheme://host/path` with `*` for the scheme, a leading `*.` host).
pub(super) fn pattern_matches(p: &str, url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    if !matches!(u.scheme(), "http" | "https") {
        return false;
    }
    if p == "<all_urls>" {
        return true;
    }
    let Some((scheme, rest)) = p.split_once("://") else {
        return false;
    };
    if scheme != "*" && scheme != u.scheme() {
        return false;
    }
    let host_part = rest.split('/').next().unwrap_or_default();
    let host = u.host_str().unwrap_or_default();
    host_part == "*"
        || host_part == host
        || host_part
            .strip_prefix("*.")
            .is_some_and(|h| host == h || host.ends_with(&format!(".{h}")))
}

fn not_found() -> MadoError {
    MadoError::new(ErrorCode::NotFound, "error.extension.notFound")
}

fn protected(origin: &Origin) -> Result<(), MadoError> {
    if origin.is_user() {
        Ok(())
    } else {
        Err(MadoError::new(
            ErrorCode::Protected,
            "error.extension.notAllowed",
        ))
    }
}

impl Core {
    fn installed(&self, id: &ExtensionId) -> Result<InstalledExtension, MadoError> {
        self.state
            .extensions
            .installed
            .iter()
            .find(|e| &e.id == id)
            .cloned()
            .ok_or_else(not_found)
    }

    /// Writes an extension's row and state, and reloads it in the engine
    /// when `reload`.
    pub(super) fn save_installed(
        &mut self,
        e: InstalledExtension,
        reload: bool,
        rec: &mut PatchRecorder,
    ) {
        let was_on = self
            .state
            .extensions
            .installed
            .iter()
            .any(|x| x.id == e.id && x.enabled);
        if was_on && !e.enabled {
            self.extension_turned_off(&e.id, false, rec);
        }
        let mut ext = self.state.extensions.clone();
        if let Some(slot) = ext.installed.iter_mut().find(|x| x.id == e.id) {
            *slot = e.clone();
        }
        let pinned = ext.pinned.contains(&e.id);
        self.state.tx(rec).set_extensions(ext);
        if reload {
            if e.enabled {
                self.load_installed(&e);
            } else if self.extension_loads.remove(&e.id).is_some() {
                self.outbox
                    .push(EngineOp::UnloadExtension(e.id.to_string()));
            }
        }
        let now = self.now();
        let row = (
            e.id.to_string(),
            e.version.clone(),
            e.name.clone(),
            e.enabled,
            e.disabled_reason.clone(),
            e.site_access.column(),
            json!(e.permissions).to_string(),
            json!(e.hosts).to_string(),
            e.private_allowed,
            pinned,
        );
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "UPDATE extensions SET version = ?2, name = ?3, enabled = ?4, disabled_reason = ?5,
                   site_access = ?6, granted_permissions = ?7, granted_hosts = ?8, private_allowed = ?9,
                   pinned = ?10, updated_at = ?11 WHERE id = ?1",
                params![row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, row.8, row.9, now],
            )
            .map(|_| ())
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "an extension row was not saved");
            }
        });
        self.refresh_toolbar(rec);
        if !was_on && e.enabled {
            self.extension_turned_on(&e.id, rec);
        }
    }

    fn developer_mode(&self) -> Result<(), MadoError> {
        if self.setting("extensions.developerMode") == true {
            Ok(())
        } else {
            Err(MadoError::new(
                ErrorCode::InvalidInput,
                "error.extension.developerMode",
            ))
        }
    }

    /// The management commands of 09 §11 and the toolbar's.
    pub(super) fn extension_manage(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
        reply_to: &mut Option<Reply>,
    ) -> Result<Value, MadoError> {
        match cmd {
            Command::ExtensionList => Ok(json!(self.state.extensions.installed)),
            Command::ExtensionSetEnabled { id, enabled } => {
                protected(origin)?;
                let mut e = self.installed(&id)?;
                if enabled && mado_lists::compat().blocked.contains_key(id.as_str()) {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "notice.extensionBlocked",
                    ));
                }
                e.enabled = enabled;
                if enabled {
                    e.disabled_reason = None;
                }
                self.save_installed(e, true, rec);
                Ok(Value::Null)
            }
            Command::ExtensionRemove { id, delete_data } => {
                protected(origin)?;
                self.installed(&id)?;
                if delete_data && self.extension_loads.contains_key(&id) {
                    self.outbox
                        .push(EngineOp::Extension(ExtensionOp::DeleteData {
                            id: id.to_string(),
                        }));
                }
                self.remove_extension(&id, rec);
                Ok(Value::Null)
            }
            Command::ExtensionSetSiteAccess { id, mode, hosts } => {
                protected(origin)?;
                let mut e = self.installed(&id)?;
                let requested = self
                    .extension_info
                    .get(&id)
                    .map(|i| i.requested_hosts.clone())
                    .unwrap_or_default();
                e.site_access = mode;
                match (mode, hosts) {
                    (SiteAccessMode::Specific, Some(hosts)) => {
                        if let Some(bad) = hosts.iter().find(|h| !requested.contains(h)) {
                            return Err(MadoError::new(
                                ErrorCode::InvalidInput,
                                "error.extension.notRequested",
                            )
                            .arg("host", bad.clone()));
                        }
                        e.hosts = hosts;
                    }
                    (SiteAccessMode::All, _) => e.hosts = requested,
                    _ => {}
                }
                self.save_installed(e, true, rec);
                Ok(Value::Null)
            }
            Command::ExtensionSetPrivateAllowed { id, allowed } => {
                protected(origin)?;
                let mut e = self.installed(&id)?;
                e.private_allowed = allowed;
                self.save_installed(e, false, rec);
                Ok(Value::Null)
            }
            Command::ExtensionGrantOptional { id, permissions } => {
                protected(origin)?;
                let mut e = self.installed(&id)?;
                for p in permissions {
                    if !e.optional.contains(&p) {
                        return Err(MadoError::new(
                            ErrorCode::InvalidInput,
                            "error.extension.notOptional",
                        )
                        .arg("permission", p));
                    }
                    let list = if p.contains("://") || p == "<all_urls>" {
                        &mut e.hosts
                    } else {
                        &mut e.permissions
                    };
                    if !list.contains(&p) {
                        list.push(p);
                    }
                }
                self.save_installed(e, true, rec);
                Ok(Value::Null)
            }
            Command::ExtensionRevokeOptional { id, permissions } => {
                protected(origin)?;
                let mut e = self.installed(&id)?;
                let optional = e.optional.clone();
                e.permissions
                    .retain(|p| !(permissions.contains(p) && optional.contains(p)));
                e.hosts
                    .retain(|p| !(permissions.contains(p) && optional.contains(p)));
                self.save_installed(e, true, rec);
                Ok(Value::Null)
            }
            Command::ExtensionSetShortcut { id, command, keys } => {
                let mut e = self.installed(&id)?;
                if !e.shortcuts.iter().any(|s| s.command == command) {
                    return Err(MadoError::new(
                        ErrorCode::NotFound,
                        "error.extension.noCommand",
                    ));
                }
                let keys = keys
                    .map(|k| mado_settings::normalize_shortcut(&k))
                    .filter(|k| !k.is_empty());
                if let Some(k) = &keys {
                    if let Some(def) = mado_settings::REGISTRY.iter().find(|d| {
                        d.key.starts_with("shortcuts.")
                            && self
                                .settings
                                .value(d)
                                .as_str()
                                .map(mado_settings::normalize_shortcut)
                                .as_deref()
                                == Some(k.as_str())
                    }) {
                        return Err(MadoError::new(
                            ErrorCode::Conflict,
                            "error.extension.shortcutTaken",
                        )
                        .arg("holder", self.settings.strings().get(def.label).to_owned()));
                    }
                    if let Some(other) = self.state.extensions.installed.iter().find(|x| {
                        x.shortcuts.iter().any(|s| {
                            s.keys.as_deref() == Some(k.as_str())
                                && !(x.id == id && s.command == command)
                        })
                    }) {
                        return Err(MadoError::new(
                            ErrorCode::Conflict,
                            "error.extension.shortcutTaken",
                        )
                        .arg("holder", other.name.clone()));
                    }
                }
                if let Some(s) = e.shortcuts.iter_mut().find(|s| s.command == command) {
                    s.keys.clone_from(&keys);
                }
                let row = (id.to_string(), command, keys);
                let pending = self.profile_db.writer().write(move |c| {
                    c.execute(
                        "INSERT INTO extension_shortcuts (extension_id, command, keys, global) VALUES (?1, ?2, ?3, 0)
                         ON CONFLICT(extension_id, command) DO UPDATE SET keys = ?3",
                        params![row.0, row.1, row.2],
                    )
                    .map(|_| ())
                });
                tokio::task::spawn_local(async move {
                    if let Err(e) = pending.await {
                        tracing::error!(error = %e, "an extension shortcut was not saved");
                    }
                });
                self.save_installed(e, false, rec);
                Ok(Value::Null)
            }
            Command::ExtensionPin { id, pinned } => {
                let e = self.installed(&id)?;
                let mut ext = self.state.extensions.clone();
                ext.pinned.retain(|p| p != &id);
                if pinned {
                    ext.pinned.push(id);
                }
                self.state.tx(rec).set_extensions(ext);
                self.save_installed(e, false, rec);
                Ok(Value::Null)
            }
            Command::ExtensionReload { id } => {
                let mut e = self.installed(&id)?;
                if !matches!(e.source, ExtensionSource::Unpacked | ExtensionSource::Zip) {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.extension.notUnpacked",
                    ));
                }
                let path = self
                    .extension_info
                    .get(&id)
                    .map(|i| i.path.clone())
                    .unwrap_or_default();
                let m = mado_extensions::Manifest::read(&path, "en").map_err(|r| {
                    MadoError::new(ErrorCode::InvalidInput, "notice.extensionMalformed")
                        .arg("reason", r.to_string())
                })?;
                let keep: Vec<(String, Option<String>)> = e
                    .shortcuts
                    .iter()
                    .map(|s| (s.command.clone(), s.keys.clone()))
                    .collect();
                let (optional, shortcuts, info) = super::extensions::describe(&m, path, &keep);
                e.name = m.name.clone();
                e.version = m.version.clone();
                e.optional = optional;
                e.shortcuts = shortcuts;
                e.has_options = info.options_page.is_some();
                self.extension_info.insert(id.clone(), info);
                self.save_installed(e, true, rec);
                Ok(Value::Null)
            }
            Command::ExtensionErrors { id } => {
                self.developer_mode()?;
                self.installed(&id)?;
                if let Some(reply) = reply_to.take() {
                    self.outbox.push(EngineOp::Extension(ExtensionOp::Errors {
                        id: id.to_string(),
                        reply: Box::new(move |errors| {
                            let _ = reply.send(Ok(json!(errors)));
                        }),
                    }));
                }
                Ok(Value::Null)
            }
            Command::ExtensionInspect { id, target } => {
                self.developer_mode()?;
                self.installed(&id)?;
                if !matches!(target.as_str(), "background" | "popup") {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.command.badArgument",
                    ));
                }
                self.outbox
                    .push(EngineOp::Extension(ExtensionOp::Inspectable {
                        id: id.to_string(),
                        on: true,
                    }));
                Ok(json!({ "inspector": "safariDevelopMenu" }))
            }
            Command::ExtensionOpenPopup {
                id,
                window_id,
                anchor,
            } => {
                let e = self.installed(&id)?;
                if !e.enabled {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.extension.disabled",
                    ));
                }
                let tab = self
                    .state
                    .windows
                    .get(&window_id)
                    .and_then(|w| w.active_tab_id)
                    .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.window.notFound"))?;
                self.outbox.push(EngineOp::Extension(ExtensionOp::Action {
                    id: id.to_string(),
                    window: window_id,
                    tab,
                    anchor,
                }));
                Ok(Value::Null)
            }
            Command::ExtensionOpenOptions { id } => {
                self.installed(&id)?;
                let info = self.extension_info.get(&id).cloned().unwrap_or_default();
                let (Some(base), Some(page)) = (info.base_url, info.options_page) else {
                    return Err(MadoError::new(
                        ErrorCode::NotFound,
                        "error.extension.noOptions",
                    ));
                };
                let url = format!(
                    "{}/{}",
                    base.trim_end_matches('/'),
                    page.trim_start_matches('/')
                );
                let window = self.current_window();
                self.open_urls(
                    &[url],
                    mado_types::command::OpenDisposition::NewTabs,
                    window,
                )?;
                Ok(Value::Null)
            }
            _ => Err(MadoError::new(
                ErrorCode::InvalidInput,
                "error.command.unknown",
            )),
        }
    }

    pub(super) fn extension_loaded(&mut self, id: &str, base_url: String) {
        if let Ok(id) = id.parse::<ExtensionId>()
            && let Some(info) = self.extension_info.get_mut(&id)
        {
            info.base_url = Some(base_url);
        }
    }

    pub(super) fn extension_actions(
        &mut self,
        tab: mado_types::TabId,
        actions: Vec<mado_engine::ExtensionActionInfo>,
        rec: &mut PatchRecorder,
    ) {
        self.extension_actions = (Some(tab), actions);
        self.refresh_toolbar(rec);
    }

    /// `state.extensions.toolbar` for the focused window's active tab
    /// (09 §1): enabled extensions with an action, pinned first.
    pub(super) fn refresh_toolbar(&mut self, rec: &mut PatchRecorder) {
        let (tab, actions) = &self.extension_actions;
        let url = tab
            .and_then(|t| self.model.tabs.get(&t))
            .map(|t| t.url.clone())
            .unwrap_or_default();
        let ext = &self.state.extensions;
        let mut toolbar: Vec<ExtensionButton> = ext
            .installed
            .iter()
            .filter(|e| e.enabled && self.extension_info.get(&e.id).is_some_and(|i| i.has_action))
            .map(|e| {
                let action = actions.iter().find(|a| a.id == e.id.as_str());
                let access = match e.site_access {
                    SiteAccessMode::OnClick => SiteAccess::OnClick,
                    _ if e.hosts.iter().any(|h| pattern_matches(h, &url)) => SiteAccess::Granted,
                    _ => SiteAccess::None,
                };
                ExtensionButton {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    icon_url: action.and_then(|a| a.icon.clone()).unwrap_or_default(),
                    badge_text: action.map(|a| a.badge.clone()).unwrap_or_default(),
                    badge_color: String::new(),
                    title: action
                        .map(|a| a.label.clone())
                        .filter(|l| !l.is_empty())
                        .unwrap_or_else(|| e.name.clone()),
                    has_popup: action.is_some_and(|a| a.has_popup),
                    enabled: action.is_none_or(|a| a.enabled),
                    site_access_for_active_tab: access,
                }
            })
            .collect();
        toolbar.sort_by_key(|b| {
            ext.pinned
                .iter()
                .position(|p| *p == b.id)
                .unwrap_or(usize::MAX)
        });
        if toolbar != self.state.extensions.toolbar {
            self.state.tx(rec).extensions().set_toolbar(toolbar);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pattern_matches;

    #[test]
    fn match_patterns() {
        assert!(pattern_matches("<all_urls>", "https://a.example/x"));
        assert!(pattern_matches("*://*/*", "http://b.example/"));
        assert!(pattern_matches(
            "https://*.example.com/*",
            "https://news.example.com/a"
        ));
        assert!(pattern_matches(
            "https://*.example.com/*",
            "https://example.com/"
        ));
        assert!(!pattern_matches(
            "https://*.example.com/*",
            "http://example.com/"
        ));
        assert!(!pattern_matches(
            "https://a.example/*",
            "https://b.example/"
        ));
        assert!(!pattern_matches("<all_urls>", "mado://newtab"));
    }
}
