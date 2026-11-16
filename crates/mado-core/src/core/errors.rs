//! Error pages, HTTPS-first and captive portals in the core (spec 05 §5,
//! §6, §8). Error pages are static HTML shown in the tab with the failed
//! URL kept; their buttons are `mado://error-action` links carrying a
//! one-time token, so a site cannot press them.

use std::collections::BTreeMap;
use std::time::Duration;

use mado_engine::{EngineOp, NavDecision, NavKind, NavRequest, NetError, TabCommand};
use mado_types::TabId;
use rusqlite::params;

use super::{Core, Internal, lookup_def};
use crate::navigation::HttpsFirst;
use crate::timers::Timer;

/// An HTTPS attempt that has not committed by now falls back (05 §5).
pub const UPGRADE_WAIT: Duration = Duration::from_secs(3);
/// `httpAllowed` lasts this long.
pub(super) const HTTP_ALLOWED_MS: i64 = 30 * 86_400_000;
/// Apple's probe answers this body when no portal is in the way.
const PROBE_SUCCESS: &str = "Success";

/// The kinds of 05 §6, plus the HTTPS-first fallback page. `blockedByMado`
/// comes with the blocker (M3-T02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Offline,
    DnsFailed,
    TimedOut,
    Refused,
    Reset,
    Certificate,
    /// A certificate error on an HSTS host: no Proceed (spec 08 §3).
    CertificateHsts,
    CaptivePortal,
    Crashed,
    UnsupportedScheme,
    TooManyRedirects,
    HttpsFallback,
    /// "Did you mean …?" for a lookalike domain (spec 08 §2).
    Lookalike,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Reload,
    Search,
    Continue,
    SignIn,
    ClearCookies,
    /// Go to the site a lookalike imitates.
    GoTo,
    /// Accept a bad certificate for this session.
    Proceed,
    /// Show the certificate that failed (08 §3).
    ViewCertificate,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Self::Reload => "reload",
            Self::Search => "search",
            Self::Continue => "continue",
            Self::SignIn => "signIn",
            Self::ClearCookies => "clearCookies",
            Self::GoTo => "goTo",
            Self::Proceed => "proceed",
            Self::ViewCertificate => "viewCertificate",
        }
    }

    fn from_name(s: &str) -> Option<Self> {
        [
            Self::Reload,
            Self::Search,
            Self::Continue,
            Self::SignIn,
            Self::ClearCookies,
            Self::GoTo,
            Self::Proceed,
            Self::ViewCertificate,
        ]
        .into_iter()
        .find(|a| a.name() == s)
    }
}

impl ErrorKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::DnsFailed => "dnsFailed",
            Self::TimedOut => "timedOut",
            Self::Refused => "refused",
            Self::Reset => "reset",
            Self::Certificate => "certificate",
            Self::CertificateHsts => "certificateHsts",
            Self::CaptivePortal => "captivePortal",
            Self::Crashed => "crashed",
            Self::UnsupportedScheme => "unsupportedScheme",
            Self::TooManyRedirects => "tooManyRedirects",
            Self::HttpsFallback => "httpsFallback",
            Self::Lookalike => "lookalike",
            Self::Failed => "failed",
        }
    }

    /// `None` for a cancelled load, which shows nothing.
    pub fn of(error: &NetError) -> Option<Self> {
        Some(match error {
            NetError::Offline => Self::Offline,
            NetError::DnsFailed => Self::DnsFailed,
            NetError::TimedOut => Self::TimedOut,
            NetError::Refused => Self::Refused,
            NetError::Reset => Self::Reset,
            NetError::Certificate => Self::Certificate,
            NetError::UnsupportedScheme => Self::UnsupportedScheme,
            NetError::TooManyRedirects => Self::TooManyRedirects,
            NetError::Cancelled => return None,
            NetError::Other(_) => Self::Failed,
        })
    }

    fn actions(self) -> &'static [Action] {
        match self {
            Self::DnsFailed => &[Action::Reload, Action::Search],
            Self::HttpsFallback => &[Action::Continue],
            Self::Lookalike => &[Action::GoTo, Action::Continue],
            Self::CaptivePortal => &[Action::SignIn],
            Self::TooManyRedirects => &[Action::ClearCookies],
            Self::Certificate => &[Action::Reload, Action::Proceed, Action::ViewCertificate],
            Self::CertificateHsts => &[Action::ViewCertificate],
            Self::UnsupportedScheme => &[],
            _ => &[Action::Reload],
        }
    }
}

/// An error page on screen.
#[derive(Debug, Clone)]
pub struct ErrorPage {
    token: String,
    kind: ErrorKind,
    url: String,
    /// The captive portal, or the site a lookalike imitates.
    portal: Option<String>,
    /// Mado navigated away from it (a button, Reload): the next commit is a
    /// page, not this one again.
    left: bool,
}

/// An HTTPS-first upgrade in flight.
#[derive(Debug, Clone)]
pub struct Upgrade {
    http: String,
    https: String,
    at: i64,
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The page's HTML: no script, no outside resources.
pub fn render(kind: ErrorKind, host: &str, token: &str, text: &dyn Fn(&str) -> String) -> String {
    let fill = |key: &str| escape(&text(key).replace("{host}", host));
    let title = fill(&format!("errorPage.{}.title", kind.key()));
    let body = fill(&format!("errorPage.{}.body", kind.key()));
    let buttons: String = kind
        .actions()
        .iter()
        .map(|a| {
            format!(
                r#"<a class="button" href="mado://error-action/?t={token}&amp;a={}">{}</a>"#,
                a.name(),
                fill(&format!("errorPage.action.{}", a.name()))
            )
        })
        .collect();
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'"><meta name="color-scheme" content="light dark"><title>{title}</title><style>body{{font:15px -apple-system,system-ui,sans-serif;max-width:36em;margin:15vh auto;padding:0 1.5em;line-height:1.5}}h1{{font-size:1.4em}}.button{{display:inline-block;margin:.5em .5em 0 0;padding:.4em 1em;border-radius:6px;background:#0a66d8;color:#fff;text-decoration:none}}</style></head><body data-kind="{}"><h1>{title}</h1><p>{body}</p>{buttons}</body></html>"#,
        kind.key()
    )
}

/// `httpAllowed` hosts with their expiry, from `site_settings`.
pub fn load_http_allowed(
    c: &rusqlite::Connection,
    now: i64,
) -> rusqlite::Result<BTreeMap<String, i64>> {
    let mut stmt = c.prepare(
        "SELECT origin, COALESCE(expires_at, 0) FROM site_settings WHERE key = 'httpAllowed' AND (expires_at IS NULL OR expires_at > ?1)",
    )?;
    let rows = stmt.query_map([now], |r| {
        let origin: String = r.get(0)?;
        Ok((host_of(&origin), r.get::<_, i64>(1)?))
    })?;
    rows.collect()
}

impl Core {
    pub(super) fn https_first(&self) -> HttpsFirst {
        HttpsFirst::from_setting(
            self.settings
                .value(lookup_def("security.httpsFirst"))
                .as_str()
                .unwrap_or_default(),
        )
    }

    /// Whether `url` may load over plain HTTP without an upgrade.
    pub(super) fn http_fallback_allowed(&self, url: &url::Url) -> bool {
        if self.continue_once.contains(url.as_str()) {
            return true;
        }
        let now = self.now();
        self.https_first() == HttpsFirst::Upgrade
            && url
                .host_str()
                .and_then(|h| self.http_allowed.get(h))
                .is_some_and(|exp| *exp == 0 || *exp > now)
    }

    /// The policy answer for a navigation, with HTTPS-first bookkeeping.
    pub(super) fn decide_navigation(&mut self, tab: TabId, request: &NavRequest) -> NavDecision {
        if let mado_engine::NavPhase::Response { hsts: true, .. } = request.phase
            && request.is_main_frame
        {
            self.hsts_seen.insert(host_of(&request.url));
        }
        if let Some(decision) = self.error_action(tab, &request.url) {
            return decision;
        }
        // A server that sends the upgraded load back to http: fall back.
        if request.is_main_frame
            && request.url.starts_with("http:")
            && let Some(up) = self.upgrades.get(&tab).cloned()
            && host_of(&up.https) == host_of(&request.url)
            && request.kind == NavKind::Redirect
        {
            self.upgrades.remove(&tab);
            self.https_fallback(tab, &up);
            return NavDecision::Cancel;
        }
        if let Some(decision) = self.link_with_modifiers(tab, request) {
            return decision;
        }
        if self.tab_under(tab, request) {
            return NavDecision::Cancel;
        }
        let context = crate::navigation::PolicyContext {
            https_first: self.https_first(),
            opens_externally: &|h| self.opens_in_safari(h),
            http_fallback: &|u| self.http_fallback_allowed(u),
        };
        let decision = crate::navigation::decide(request, &context);
        if decision == NavDecision::Allow && self.lookalike_check(tab, request) {
            self.continue_once.remove(&request.url);
            return NavDecision::Cancel;
        }
        match &decision {
            NavDecision::OpenExternal {
                url,
                reason: mado_engine::ExternalReason::EngineRule,
            } => self.ui.open_in_safari(url),
            NavDecision::OpenExternal {
                reason: mado_engine::ExternalReason::AppLink,
                ..
            } => self.app_link_requested(tab, request),
            _ => {}
        }
        if request.is_main_frame {
            self.continue_once.remove(&request.url);
            if let NavDecision::Redirect { url } = &decision
                && request.url.starts_with("http:")
            {
                let at = self.now();
                self.upgrades.insert(
                    tab,
                    Upgrade {
                        http: request.url.clone(),
                        https: url.clone(),
                        at,
                    },
                );
                self.timers
                    .schedule(Timer::UpgradeTimeout(tab), UPGRADE_WAIT);
            }
        }
        decision
    }

    /// The failed URL under `tab`'s error page, while it is on screen.
    pub(super) fn error_page_url(&self, tab: TabId) -> Option<String> {
        self.error_pages
            .get(&tab)
            .filter(|p| !p.left)
            .map(|p| p.url.clone())
    }

    /// Mado is about to load something in `tab`: its error page, if any,
    /// is left.
    pub(super) fn leave_error_page(&mut self, tab: TabId) {
        if let Some(page) = self.error_pages.get_mut(&tab) {
            page.left = true;
        }
    }

    /// The error page's own link for `action`, followed (test hook).
    #[cfg(any(test, feature = "test-hooks"))]
    pub(super) fn press_error_action(&mut self, tab: TabId, action: &str) {
        if let Some(page) = self.error_pages.get(&tab).cloned() {
            let link = format!("mado://error-action/?t={}&a={action}", page.token);
            let _ = self.error_action(tab, &link);
        }
    }

    /// 08 §2: a first visit to a host that imitates a well-known or frequent
    /// site shows "Did you mean …?" instead. Returns true when shown.
    fn lookalike_check(&mut self, tab: TabId, request: &NavRequest) -> bool {
        if !request.is_main_frame
            || !matches!(request.phase, mado_engine::NavPhase::Action { .. })
            || self.continue_once.contains(&request.url)
            || self.pending_transition.get(&tab) == Some(&crate::history::Transition::Typed)
        {
            return false;
        }
        let Ok(url) = url::Url::parse(&request.url) else {
            return false;
        };
        if !matches!(url.scheme(), "http" | "https") {
            return false;
        }
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
            return false;
        };
        let Some(origin) = mado_permissions::origin_of(&request.url) else {
            return false;
        };
        if self.site_value(&origin, "lookalikeIgnored", None) == mado_permissions::Value::Allow {
            return false;
        }
        let readers = self.profile_db.readers();
        let seen = readers
            .read(|c| {
                c.query_row(
                    "SELECT EXISTS (SELECT 1 FROM urls WHERE host = ?1 AND visit_count > 0)
                         OR EXISTS (SELECT 1 FROM bookmarks WHERE url LIKE '%://' || ?1 || '/%' OR url LIKE '%://' || ?1)",
                    [&host],
                    |r| r.get::<_, bool>(0),
                )
            })
            .unwrap_or(true);
        if seen {
            return false;
        }
        let frequent: Vec<String> = readers
            .read(|c| {
                let mut stmt = c.prepare_cached(
                    "SELECT host FROM urls GROUP BY host HAVING SUM(visit_count) >= 5 LIMIT 500",
                )?;
                stmt.query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|h: String| psl::domain_str(&h).map_or_else(|| h.clone(), str::to_owned))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let known = mado_safety::Known {
            frequent: &frequent,
        };
        let Some(target) =
            mado_safety::lookalike(&host, &known, &mado_navigation::TopDomains::current())
        else {
            return false;
        };
        self.show_error(tab, ErrorKind::Lookalike, request.url.clone(), Some(target));
        true
    }

    /// 05 §7: a link clicked with modifiers opens in a new tab or window,
    /// or downloads, and the page stays.
    fn link_with_modifiers(&mut self, tab: TabId, request: &NavRequest) -> Option<NavDecision> {
        let mado_engine::NavPhase::Action {
            modifiers: m,
            method,
        } = &request.phase
        else {
            return None;
        };
        if !request.is_main_frame
            || request.initiated_by_mado
            || !request.user_gesture
            || method != "GET"
            || !(request.url.starts_with("http:") || request.url.starts_with("https:"))
        {
            return None;
        }
        if m.option && !m.command && !m.shift {
            return Some(NavDecision::Download);
        }
        let new_tab = m.command || m.button == 1;
        if !new_tab && !m.shift {
            return None;
        }
        let window = self.model.tabs.get(&tab)?.window_id;
        let (now, prefs) = (self.now(), self.prefs());
        let opened = if new_tab {
            self.model
                .open_tab(
                    window,
                    request.url.clone(),
                    None,
                    m.shift,
                    Some(tab),
                    now,
                    prefs,
                )
                .ok()
        } else {
            self.open_window(
                mado_types::state::Bounds::default(),
                std::slice::from_ref(&request.url),
            )
            .ok()
            .and_then(|w| {
                self.model
                    .window(w)
                    .ok()
                    .and_then(|w| w.tab_order.first().copied())
            })
        };
        if let Some(t) = opened {
            self.pending_transition
                .insert(t, crate::history::Transition::Link);
        }
        Some(NavDecision::Cancel)
    }

    /// 05 §7, tab-under: within a second of opening a pop-up, the opener may
    /// not move to another site without a gesture of its own.
    fn tab_under(&mut self, tab: TabId, request: &NavRequest) -> bool {
        if !request.is_main_frame || request.initiated_by_mado || request.user_gesture {
            return false;
        }
        let now = self.now();
        let Some(at) = self.popup_opened.get(&tab).copied() else {
            return false;
        };
        if now - at > 1000 {
            self.popup_opened.remove(&tab);
            return false;
        }
        let site = |u: &str| {
            let host = host_of(u);
            psl::domain_str(&host).map_or(host.clone(), str::to_owned)
        };
        let current = self
            .model
            .tabs
            .get(&tab)
            .map(|t| t.url.clone())
            .unwrap_or_default();
        site(&current) != site(&request.url)
    }

    /// A main-frame document committed: an upgrade succeeded, and an
    /// error page commit is not a visit. Returns true for an error page.
    pub(super) fn committed_page(&mut self, tab: TabId, url: &str) -> bool {
        // A site whose certificate the user accepted stays marked (08 §3).
        if url.starts_with("https:")
            && self.proceeded.contains(&host_of(url))
            && let Some(t) = self.model.tabs.get_mut(&tab)
        {
            t.security.level = mado_types::state::SecurityLevel::Dangerous;
        }
        if url.starts_with("https:")
            && self
                .upgrades
                .get(&tab)
                .is_some_and(|u| host_of(&u.https) == host_of(url))
        {
            self.upgrades.remove(&tab);
        }
        // WebKit can commit a simulated load more than once: every commit of
        // the failed URL until Mado navigates is the error page.
        match self.error_pages.get(&tab) {
            Some(page) if !page.left && page.url == url => true,
            Some(_) => {
                self.error_pages.remove(&tab);
                false
            }
            None => false,
        }
    }

    pub(super) fn upgrade_timed_out(&mut self, tab: TabId) {
        let now = self.now();
        let Some(up) = self.upgrades.get(&tab).cloned() else {
            return;
        };
        if now - up.at < i64::try_from(UPGRADE_WAIT.as_millis()).unwrap_or(3000) {
            return;
        }
        self.upgrades.remove(&tab);
        self.outbox.push(EngineOp::Apply(tab, TabCommand::Stop));
        self.https_fallback(tab, &up);
    }

    fn https_fallback(&mut self, tab: TabId, up: &Upgrade) {
        let allowed = url::Url::parse(&up.http).is_ok_and(|u| self.http_fallback_allowed(&u));
        if allowed {
            self.continue_once.insert(up.http.clone());
            let _ = self.navigate(tab, up.http.clone());
        } else {
            self.show_error(tab, ErrorKind::HttpsFallback, up.http.clone(), None);
        }
    }

    pub(super) fn navigation_failed(&mut self, tab: TabId, url: String, error: &NetError) {
        // An engine that could not say which URL failed: the tab's.
        let url = if url.is_empty() {
            self.model
                .tabs
                .get(&tab)
                .map(|t| t.url.clone())
                .unwrap_or_default()
        } else {
            url
        };
        let Some(mut kind) = ErrorKind::of(error) else {
            return;
        };
        // 08 §3: no Proceed on hosts that asked for HSTS, now or by the
        // preload list.
        let host = host_of(&url);
        if kind == ErrorKind::Certificate
            && (self.hsts_seen.contains(&host)
                || mado_safety::hsts_preloaded(&host)
                || self.hsts_extra.contains(&host))
        {
            kind = ErrorKind::CertificateHsts;
        }
        if let Some(up) = self.upgrades.get(&tab).cloned()
            && host_of(&up.https) == host_of(&url)
            && matches!(
                kind,
                ErrorKind::TimedOut
                    | ErrorKind::Refused
                    | ErrorKind::Reset
                    | ErrorKind::Certificate
                    | ErrorKind::Failed
            )
        {
            self.upgrades.remove(&tab);
            self.https_fallback(tab, &up);
            return;
        }
        if matches!(
            kind,
            ErrorKind::TimedOut
                | ErrorKind::Reset
                | ErrorKind::Refused
                | ErrorKind::DnsFailed
                | ErrorKind::Failed
        ) {
            self.probe_captive_portal(tab, url.clone());
        }
        self.show_error(tab, kind, url, None);
    }

    /// The tab's content process went away: "This page crashed".
    pub(super) fn page_crashed(&mut self, tab: TabId) {
        if let Some(url) = self.model.tabs.get(&tab).map(|t| t.url.clone()) {
            self.show_error(tab, ErrorKind::Crashed, url, None);
        }
    }

    pub(super) fn show_error(
        &mut self,
        tab: TabId,
        kind: ErrorKind,
        url: String,
        portal: Option<String>,
    ) {
        let token = uuid::Uuid::now_v7().simple().to_string();
        let strings = self.settings.strings();
        let target = portal.clone().unwrap_or_default();
        let html = render(kind, &host_of(&url), &token, &|k| {
            strings.get(k).replace("{suggestion}", &target)
        });
        self.error_pages.insert(
            tab,
            ErrorPage {
                token,
                kind,
                url: url.clone(),
                portal,
                left: false,
            },
        );
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::ShowErrorPage { html, url },
        ));
    }

    /// A button on an error page, recognised by its token; anything else
    /// under `mado://error-action` is cancelled.
    fn error_action(&mut self, tab: TabId, url: &str) -> Option<NavDecision> {
        if !url.starts_with("mado://error-action") {
            return None;
        }
        let parsed = url::Url::parse(url).ok();
        let arg = |name: &str| {
            parsed.as_ref().and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.into_owned())
            })
        };
        let page = self.error_pages.get(&tab).cloned();
        let (Some(page), Some(token), Some(action)) = (
            page,
            arg("t"),
            arg("a").as_deref().and_then(Action::from_name),
        ) else {
            return Some(NavDecision::Cancel);
        };
        if token != page.token || !page.kind.actions().contains(&action) {
            return Some(NavDecision::Cancel);
        }
        let host = host_of(&page.url);
        match action {
            Action::Reload => {
                let _ = self.navigate(tab, page.url.clone());
            }
            Action::Search => {
                let id = self.settings.value(lookup_def("search.defaultEngine"));
                let id = id.as_str().unwrap_or_default().to_owned();
                if let Some(e) = self.engines.iter().find(|e| e.active && e.engine.id == id) {
                    let to = mado_navigation::engines::expand(&e.engine.search_url, &host);
                    let _ = self.navigate(tab, to);
                }
            }
            Action::Continue if page.kind == ErrorKind::Lookalike => {
                self.write_site(
                    &format!("[*.]{host}"),
                    "lookalikeIgnored",
                    None,
                    mado_permissions::Value::Allow,
                    mado_permissions::Source::User,
                );
                self.continue_once.insert(page.url.clone());
                let _ = self.navigate(tab, page.url.clone());
            }
            Action::Continue => {
                let now = self.now();
                let expires = now + HTTP_ALLOWED_MS;
                self.http_allowed.insert(host.clone(), expires);
                self.continue_once.insert(page.url.clone());
                if !self.state.profile.is_private {
                    let origin = format!("http://{host}");
                    let pending = self.profile_db.writer().write(move |c| {
                        c.execute(
                            "INSERT OR REPLACE INTO site_settings (origin, key, value, embedder, granted_at, expires_at, source)
                             VALUES (?1, 'httpAllowed', 'allow', '', ?2, ?3, 'user')",
                            params![origin, now, expires],
                        )
                        .map(|_| ())
                    });
                    tokio::task::spawn_local(async move {
                        if let Err(e) = pending.await {
                            tracing::error!(error = %e, "httpAllowed was not saved");
                        }
                    });
                }
                let _ = self.navigate(tab, page.url.clone());
            }
            Action::SignIn => {
                if let (Some(portal), Some(window)) = (
                    page.portal.clone(),
                    self.model.tabs.get(&tab).map(|t| t.window_id),
                ) {
                    let (now, prefs) = (self.now(), self.prefs());
                    let _ = self
                        .model
                        .open_tab(window, portal, None, true, None, now, prefs);
                }
            }
            Action::ViewCertificate => {
                self.outbox.push(EngineOp::Apply(
                    tab,
                    mado_engine::TabCommand::ShowCertificate,
                ));
            }
            Action::Proceed => {
                self.proceeded.insert(host.clone());
                self.outbox.push(EngineOp::AllowCertificate(host));
                let _ = self.navigate(tab, page.url.clone());
            }
            Action::GoTo => {
                if let Some(target) = page.portal.clone() {
                    let _ = self.navigate(tab, format!("https://{target}/"));
                }
            }
            Action::ClearCookies => {
                let request = mado_engine::ClearDataRequest {
                    kinds: mado_engine::DataKinds {
                        cookies: true,
                        ..mado_engine::DataKinds::default()
                    },
                    since: None,
                    hosts: Some(vec![host]),
                };
                let inputs = self.inputs.clone();
                let url = page.url.clone();
                self.outbox.push(EngineOp::ClearData(
                    request,
                    Box::new(move |_| {
                        let _ = inputs.send(super::Input::Internal(Internal::Reload { tab, url }));
                    }),
                ));
            }
        }
        Some(NavDecision::Cancel)
    }

    /// After a failed load, asks whether a captive portal is in the way
    /// (05 §8); a portal turns the error page into "Sign in to the network".
    fn probe_captive_portal(&mut self, tab: TabId, url: String) {
        let Some(probe) = self.captive_probe.clone() else {
            return;
        };
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build();
            let Ok(client) = client else { return };
            let Ok(response) = client.get(&probe).send().await else {
                return;
            };
            let portal = response.url().to_string();
            let Ok(body) = response.text().await else {
                return;
            };
            if body.contains(PROBE_SUCCESS) {
                return;
            }
            let _ = inputs.send(super::Input::Internal(Internal::CaptivePortal {
                tab,
                url,
                portal,
            }));
        });
    }

    pub(super) fn captive_portal(&mut self, tab: TabId, url: String, portal: String) {
        if self.error_pages.get(&tab).is_some_and(|p| p.url == url) {
            self.show_error(tab, ErrorKind::CaptivePortal, url, Some(portal));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_escape_the_host_and_carry_only_their_actions() {
        let html = render(ErrorKind::DnsFailed, "<x>.example", "tok", &|k| match k {
            "errorPage.dnsFailed.title" => "Can't find {host}".into(),
            other => other.into(),
        });
        assert!(html.contains("Can&#39;t find &lt;x&gt;.example"));
        assert!(
            html.contains("a=reload") && html.contains("a=search") && !html.contains("a=continue")
        );
        assert!(!html.contains("<script"));
    }

    #[test]
    fn every_page_and_button_has_its_strings() {
        let strings = mado_settings::Strings::english();
        let kinds = [
            ErrorKind::Offline,
            ErrorKind::DnsFailed,
            ErrorKind::TimedOut,
            ErrorKind::Refused,
            ErrorKind::Reset,
            ErrorKind::Certificate,
            ErrorKind::CertificateHsts,
            ErrorKind::CaptivePortal,
            ErrorKind::Crashed,
            ErrorKind::UnsupportedScheme,
            ErrorKind::TooManyRedirects,
            ErrorKind::HttpsFallback,
            ErrorKind::Lookalike,
            ErrorKind::Failed,
        ];
        for kind in kinds {
            for part in ["title", "body"] {
                let key = format!("errorPage.{}.{part}", kind.key());
                assert!(strings.contains(&key), "{key}");
            }
            for a in kind.actions() {
                let key = format!("errorPage.action.{}", a.name());
                assert!(strings.contains(&key), "{key}");
            }
        }
    }
}
