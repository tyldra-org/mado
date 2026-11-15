//! App links and protocol handlers in the core (spec 07 §6).

use std::collections::BTreeMap;

use mado_engine::NavRequest;
use mado_engine::page::ProtocolHandlerRegister;
use mado_types::command::PromptDecision;
use mado_types::state::{Notice, PromptState};
use mado_types::{PatchRecorder, PromptId, TabId};
use rusqlite::{Connection, params};
use serde_json::json;

use super::{Core, Internal};

/// Schemes `registerProtocolHandler` may claim (HTML's safelist), plus
/// `web+` schemes.
const SAFELISTED: &[&str] = &[
    "bitcoin", "ftp", "ftps", "geo", "im", "irc", "ircs", "magnet", "mailto", "matrix", "mms", "news",
    "nntp", "openpgp4fpr", "sftp", "sip", "sms", "smsto", "ssh", "tel", "urn", "webcal", "wtai", "xmpp",
];

/// What the core resolved about the app before asking.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub tab: TabId,
    pub url: String,
    pub origin: String,
    pub app: Option<(String, Option<String>)>,
}

/// `app_link_rules` and default `protocol_handlers`.
pub fn load(c: &Connection) -> rusqlite::Result<(Vec<(String, String)>, BTreeMap<String, String>)> {
    let mut rules = c.prepare("SELECT origin, scheme FROM app_link_rules")?;
    let rules = rules
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut handlers = c.prepare("SELECT scheme, template FROM protocol_handlers WHERE is_default = 1")?;
    let handlers = handlers
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok((rules, handlers))
}

/// Whether a page may register `r` (HTML's rules): a safelisted or
/// `web+` scheme, and an https template on the page's own origin with `%s`.
pub fn valid_handler(r: &ProtocolHandlerRegister, page_origin: &str) -> bool {
    let scheme = r.scheme.to_ascii_lowercase();
    let scheme_ok = SAFELISTED.contains(&scheme.as_str())
        || scheme
            .strip_prefix("web+")
            .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase()));
    scheme_ok
        && r.url.contains("%s")
        && r.url.starts_with("https://")
        && mado_permissions::origin_of(&r.url.replace("%s", "x")).as_deref() == Some(page_origin)
}

fn scheme_of(url: &str) -> String {
    url.split_once(':').map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_default()
}

impl Core {
    /// 07 §6 steps 1 to 5, synchronously in the navigation decision; the
    /// handler lookup and prompt follow off the decision.
    pub(super) fn app_link_requested(&mut self, tab: TabId, request: &NavRequest) {
        let url = request.url.clone();
        let agent = self.model.tabs.get(&tab).is_some_and(|t| t.agent.is_some());
        let gesture = request.user_gesture || request.initiated_by_mado;
        if agent || mado_permissions::blocked_scheme(&url, gesture) || !gesture || !request.is_main_frame {
            return;
        }
        let now = self.now();
        if !self.launches.allow(tab, now) {
            return;
        }
        let scheme = scheme_of(&url);
        if matches!(scheme.as_str(), "mailto" | "tel")
            && let Some(template) = self.protocol_handlers.get(&scheme).cloned()
        {
            let encoded: String = url::form_urlencoded::byte_serialize(url.as_bytes()).collect();
            let to = template.replace("%s", &encoded);
            if let Some(window) = self.model.tabs.get(&tab).map(|t| t.window_id) {
                let prefs = self.prefs();
                let _ = self.model.open_tab(window, to, None, true, Some(tab), now, prefs);
            }
            return;
        }
        let origin = self
            .model
            .tabs
            .get(&tab)
            .and_then(|t| mado_permissions::origin_of(&t.url))
            .unwrap_or_default();
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || {
            let app = mado_platform::applinks::handler(&url).ok().flatten().map(|h| {
                let icon = h.icon_png.map(|png| format!("data:image/png;base64,{}", base64(&png)));
                (h.name, icon)
            });
            let _ = inputs.send(super::Input::Internal(Internal::AppLinkResolved(Resolved { tab, url, origin, app })));
        });
    }

    /// 07 §6 steps 3, 6 and 7.
    pub(super) fn app_link_resolved(&mut self, r: Resolved, rec: &mut PatchRecorder) {
        let scheme = scheme_of(&r.url);
        let Some((app_name, app_icon)) = r.app else {
            let id = format!("n{}", self.next_notice);
            self.next_notice += 1;
            let notice = Notice {
                id,
                message_key: "notice.noAppForLink".into(),
                args: BTreeMap::new(),
                action: None,
                undo: None,
                expires_at: Some(self.now() + 6000),
            };
            self.add_notice(notice, rec);
            return;
        };
        // mailto: and tel: are safe: no prompt.
        if matches!(scheme.as_str(), "mailto" | "tel") || self.app_link_rules.contains(&(r.origin.clone(), scheme.clone())) {
            self.ui.launch_app_link(&r.url);
            return;
        }
        let id = PromptId::new();
        let now = self.now();
        let mut prompts = self.state.prompts.clone();
        prompts.insert(
            id,
            PromptState::AppLink {
                id,
                tab_id: r.tab,
                origin: r.origin.clone(),
                scheme,
                app_name,
                app_icon,
                created_at: now,
                armed_at: now + super::site::ARM_MS,
            },
        );
        self.state.tx(rec).set_prompts(prompts);
        self.app_link_urls.insert(id, r.url);
    }

    /// `navigator.registerProtocolHandler`: a valid request asks.
    pub(super) fn protocol_handler_requested(&mut self, tab: TabId, r: &ProtocolHandlerRegister, rec: &mut PatchRecorder) {
        let Some(origin) = self.model.tabs.get(&tab).and_then(|t| mado_permissions::origin_of(&t.url)) else {
            return;
        };
        if self.state.profile.is_private || !valid_handler(r, &origin) {
            return;
        }
        let id = PromptId::new();
        let now = self.now();
        let mut prompts = self.state.prompts.clone();
        prompts.insert(
            id,
            PromptState::ProtocolHandler {
                id,
                tab_id: tab,
                origin,
                scheme: r.scheme.to_ascii_lowercase(),
                created_at: now,
                armed_at: now + super::site::ARM_MS,
            },
        );
        self.state.tx(rec).set_prompts(prompts);
        self.handler_templates.insert(id, r.url.clone());
    }

    /// `prompt.respond` for app-link and protocol-handler prompts.
    pub(super) fn answer_link_prompt(&mut self, id: PromptId, decision: PromptDecision, rec: &mut PatchRecorder) {
        let Some(prompt) = self.state.prompts.get(&id).cloned() else {
            return;
        };
        let mut prompts = self.state.prompts.clone();
        prompts.remove(&id);
        self.state.tx(rec).set_prompts(prompts);
        let allow = decision != PromptDecision::Never;
        let remember = decision == PromptDecision::AllowWhileVisiting;
        let now = self.now();
        match prompt {
            PromptState::AppLink { origin, scheme, .. } => {
                let Some(url) = self.app_link_urls.remove(&id) else { return };
                if !allow {
                    return;
                }
                self.ui.launch_app_link(&url);
                if remember {
                    self.app_link_rules.insert((origin.clone(), scheme.clone()));
                    self.write_profile(move |c| {
                        c.execute(
                            "INSERT OR IGNORE INTO app_link_rules (origin, scheme, created_at) VALUES (?1, ?2, ?3)",
                            params![origin, scheme, now],
                        )
                        .map(|_| ())
                    });
                }
            }
            PromptState::ProtocolHandler { origin, scheme, .. } => {
                let Some(template) = self.handler_templates.remove(&id) else { return };
                if !allow {
                    return;
                }
                let first = !self.protocol_handlers.contains_key(&scheme);
                if first {
                    self.protocol_handlers.insert(scheme.clone(), template.clone());
                }
                self.write_profile(move |c| {
                    c.execute(
                        "INSERT OR REPLACE INTO protocol_handlers (scheme, origin, template, is_default) VALUES (?1, ?2, ?3, ?4)",
                        params![scheme, origin, template, first],
                    )
                    .map(|_| ())
                });
            }
            _ => {}
        }
    }

    /// Drops a tab's link prompts (new document, closed tab).
    pub(super) fn forget_link_prompts(&mut self, tab: TabId, rec: &mut PatchRecorder) {
        let gone: Vec<PromptId> = self
            .state
            .prompts
            .iter()
            .filter(|(_, p)| {
                matches!(p, PromptState::AppLink { tab_id, .. } | PromptState::ProtocolHandler { tab_id, .. } if *tab_id == tab)
            })
            .map(|(id, _)| *id)
            .collect();
        if gone.is_empty() {
            return;
        }
        let mut prompts = self.state.prompts.clone();
        for id in &gone {
            prompts.remove(id);
            self.app_link_urls.remove(id);
            self.handler_templates.remove(id);
        }
        self.state.tx(rec).set_prompts(prompts);
    }

    fn write_profile(&self, job: impl FnOnce(&Connection) -> rusqlite::Result<()> + Send + 'static) {
        if self.state.profile.is_private {
            return;
        }
        let pending = self.profile_db.writer().write(job);
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "an app link setting was not saved");
            }
        });
    }
}

/// Standard base64, for the icon's data URL.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(T[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn handlers_must_be_safe_and_same_origin() {
        let r = |scheme: &str, url: &str| ProtocolHandlerRegister { scheme: scheme.into(), url: url.into() };
        let site = "https://mail.example";
        assert!(valid_handler(&r("mailto", "https://mail.example/compose?to=%s"), site));
        assert!(valid_handler(&r("web+coffee", "https://mail.example/c?u=%s"), site));
        assert!(!valid_handler(&r("http", "https://mail.example/c?u=%s"), site));
        assert!(!valid_handler(&r("mailto", "https://evil.example/c?u=%s"), site));
        assert!(!valid_handler(&r("mailto", "https://mail.example/compose"), site));
        assert!(!valid_handler(&r("web+Coffee", "https://mail.example/c?u=%s"), site));
    }
}
