//! Extension updates and the kill switch (spec 09 §7, §10): a daily check
//! of each store extension; an update that asks for more is installed
//! turned off for review; extensions on the compatibility list's blocked
//! list are turned off at once.

use std::time::Duration;

use mado_extensions::Expect;
use mado_extensions::store::{MAX_PACKAGE, download, newer, parse_update};
use mado_types::command::{Command, ExtensionSource};
use mado_types::patch::PatchRecorder;
use mado_types::state::{InstalledExtension, Notice, NoticeAction, SiteAccessMode};
use serde_json::json;

use super::extensions::{Staged, describe, is_all_sites_any, move_into_profile, stage_crx};
use super::{Core, Internal, Timer};

/// The first check after start, then daily.
pub(super) const FIRST: Duration = Duration::from_secs(10 * 60);
const EVERY: Duration = Duration::from_secs(24 * 3600);

/// Whether `new` (required permissions and hosts) asks for anything `e`
/// was not granted (09 §10).
pub(super) fn asks_for_more(
    e: &InstalledExtension,
    permissions: &[String],
    hosts: &[String],
) -> bool {
    let had_all = e.site_access == SiteAccessMode::All || is_all_sites_any(&e.hosts);
    permissions.iter().any(|p| !e.permissions.contains(p))
        || (!had_all && hosts.iter().any(|h| !e.hosts.contains(h)))
}

impl Core {
    fn notice_with(
        &mut self,
        key: &str,
        name: &str,
        action: Option<NoticeAction>,
        rec: &mut PatchRecorder,
    ) {
        let id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let notice = Notice {
            id,
            message_key: key.into(),
            args: std::collections::BTreeMap::from([("name".to_owned(), json!(name))]),
            action,
            undo: None,
            expires_at: None,
        };
        self.add_notice(notice, rec);
    }

    /// The daily check (09 §10): one request per store extension.
    pub(super) fn check_extension_updates(&mut self) {
        self.timers.schedule(Timer::ExtensionUpdate, EVERY);
        let Some(config) = self.extensions.clone() else {
            return;
        };
        if self.state.profile.is_private {
            return;
        }
        let blocked = mado_lists::compat().blocked.clone();
        let candidates: Vec<(String, String)> = self
            .state
            .extensions
            .installed
            .iter()
            .filter(|e| e.source == ExtensionSource::Store && !blocked.contains_key(e.id.as_str()))
            .map(|e| (e.id.to_string(), e.version.clone()))
            .collect();
        if candidates.is_empty() {
            return;
        }
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            let Ok(client) = mado_extensions::store::client() else {
                return;
            };
            for (id, version) in candidates {
                let xml = match download(&client, &(config.update_url)(&id, &version), 256 * 1024)
                    .await
                {
                    Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                    Err(e) => {
                        tracing::warn!(extension = %id, error = %e, "an update check failed");
                        continue;
                    }
                };
                let Some((offered, codebase)) = parse_update(&xml, &id) else {
                    continue;
                };
                if !newer(&offered, &version) {
                    continue;
                }
                let allowed = reqwest::Url::parse(&codebase)
                    .is_ok_and(|u| !config.check_hosts || mado_extensions::store::allowed(&u));
                if !allowed {
                    tracing::warn!(extension = %id, "an update's package is not on Google's hosts");
                    continue;
                }
                let bytes = match download(&client, &codebase, MAX_PACKAGE).await {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!(extension = %id, error = %e, "an update did not download");
                        continue;
                    }
                };
                let dir = config.dir.clone();
                let extra = config.extra_publishers.clone();
                let staged = tokio::task::spawn_blocking(move || {
                    stage_crx(
                        &bytes,
                        Expect {
                            id: Some(&id),
                            from_store: true,
                            extra_publishers: &extra,
                        },
                        &dir,
                        ExtensionSource::Store,
                    )
                })
                .await;
                match staged {
                    Ok(Ok(s)) => {
                        let _ = inputs.send(super::Input::Internal(
                            Internal::ExtensionUpdateStaged(Box::new(s)),
                        ));
                    }
                    Ok(Err(key)) => tracing::warn!(reason = key, "an update was refused"),
                    Err(_) => {}
                }
            }
        });
    }

    /// A verified update (09 §10): installed, and loaded unless it asks for
    /// more than was granted, in which case it waits turned off for Review.
    pub(super) fn extension_update_staged(&mut self, staged: Staged, rec: &mut PatchRecorder) {
        let Some(dir) = self.extensions_dir() else {
            return;
        };
        let id = staged.id.clone();
        let m = staged.manifest.clone();
        let Some(mut e) = self
            .state
            .extensions
            .installed
            .iter()
            .find(|x| x.id == id)
            .cloned()
        else {
            let _ = std::fs::remove_dir_all(&staged.files);
            return;
        };
        if !newer(&m.version, &e.version) {
            let _ = std::fs::remove_dir_all(&staged.files);
            return;
        }
        let path = match move_into_profile(&dir, &id, &m.version, &staged.files) {
            Ok(p) => p,
            Err(err) => {
                tracing::error!(error = %err, "an extension update could not be installed");
                let _ = std::fs::remove_dir_all(&staged.files);
                return;
            }
        };
        let summary = m.summary(mado_extensions::manifest::SUPPORTED_PERMISSIONS);
        let more = asks_for_more(&e, &summary.permissions, &summary.hosts);
        let keep: Vec<(String, Option<String>)> = e
            .shortcuts
            .iter()
            .map(|s| (s.command.clone(), s.keys.clone()))
            .collect();
        let (optional, shortcuts, info) = describe(&m, path, &keep);
        e.version = m.version.clone();
        e.name = m.name.clone();
        e.optional = optional;
        e.shortcuts = shortcuts;
        e.has_options = info.options_page.is_some();
        self.extension_info.insert(id.clone(), info);
        if more {
            // Turning it back on is the consent to what it now asks for.
            e.enabled = false;
            e.disabled_reason = Some("permissionsIncrease".into());
            e.permissions = summary.permissions;
            e.hosts = summary.hosts;
            if summary.all_sites {
                e.site_access = SiteAccessMode::All;
            }
            let name = e.name.clone();
            self.save_installed(e, true, rec);
            self.notice_with(
                "notice.extensionNeedsReview",
                &name,
                Some(NoticeAction {
                    label_key: "notice.review".into(),
                    command: Command::AppOpenSettings {
                        section: Some("extensions".into()),
                    },
                }),
                rec,
            );
        } else {
            tracing::info!(extension = %id, version = %e.version, "extension updated");
            self.save_installed(e, true, rec);
        }
    }

    /// The kill switch (09 §7): extensions on the blocked list turn off at
    /// once and cannot be turned on while listed.
    pub(super) fn apply_blocked_extensions(&mut self, rec: &mut PatchRecorder) {
        let blocked = mado_lists::compat().blocked.clone();
        let hit: Vec<InstalledExtension> = self
            .state
            .extensions
            .installed
            .iter()
            .filter(|e| {
                blocked.contains_key(e.id.as_str())
                    && e.disabled_reason.as_deref() != Some("blocked")
            })
            .cloned()
            .collect();
        for mut e in hit {
            e.enabled = false;
            e.disabled_reason = Some("blocked".into());
            let name = e.name.clone();
            self.save_installed(e, true, rec);
            self.notice_with("notice.extensionDisabledHarmful", &name, None, rec);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn more_is_new_permissions_or_new_hosts_beyond_all_sites() {
        let e = InstalledExtension {
            id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().expect("id"),
            name: "x".into(),
            version: "1".into(),
            enabled: true,
            source: ExtensionSource::Store,
            disabled_reason: None,
            site_access: SiteAccessMode::Specific,
            hosts: vec!["https://a.example/*".into()],
            permissions: vec!["storage".into()],
            optional: vec![],
            private_allowed: false,
            has_options: false,
            shortcuts: vec![],
        };
        let p = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(!asks_for_more(
            &e,
            &p(&["storage"]),
            &p(&["https://a.example/*"])
        ));
        assert!(asks_for_more(&e, &p(&["storage", "tabs"]), &p(&[])));
        assert!(asks_for_more(&e, &p(&["storage"]), &p(&["<all_urls>"])));
        let all = InstalledExtension {
            site_access: SiteAccessMode::All,
            hosts: p(&["<all_urls>"]),
            ..e
        };
        assert!(!asks_for_more(
            &all,
            &p(&["storage"]),
            &p(&["https://b.example/*"])
        ));
    }
}
