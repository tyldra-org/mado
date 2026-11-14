//! Navigation decisions (spec 02 §3): every main-frame and sub-frame
//! navigation passes here before it starts, and every response before it is
//! shown. Rules run in order and the first match wins.
//!
//! The per-site inputs come from their owners: "Open in Safari" rules from
//! the compatibility list (M2-T06, M3-T01) and `httpAllowed` site settings
//! from the HTTPS-first fallback page (M2-T06).

use mado_engine::{ExternalReason, NavDecision, NavPhase, NavRequest};
use url::{Host, Url};

/// Setting `security.httpsFirst` (spec 05 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HttpsFirst {
    Off,
    #[default]
    Upgrade,
    Strict,
}

impl HttpsFirst {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "off" => Self::Off,
            "strict" => Self::Strict,
            _ => Self::Upgrade,
        }
    }
}

/// What the decision needs to know beyond the request.
pub struct PolicyContext<'a> {
    pub https_first: HttpsFirst,
    /// A per-site "Open in Safari" rule matches this host.
    pub opens_externally: &'a dyn Fn(&str) -> bool,
    /// This URL is the plain-HTTP fallback the user (or, in `upgrade` mode,
    /// a host they continued to before) chose; it must not be upgraded again.
    pub http_fallback: &'a dyn Fn(&Url) -> bool,
}

impl std::fmt::Debug for PolicyContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PolicyContext")
            .field("https_first", &self.https_first)
            .finish_non_exhaustive()
    }
}

/// Schemes Mado loads itself; anything else belongs to another app.
const INTERNAL_SCHEMES: &[&str] = &[
    "http",
    "https",
    "about",
    "blob",
    "data",
    "file",
    "mado",
    "webkit-extension",
    "chrome-extension",
    "extension",
];

/// The scheme of `url`, lower case, or `None` if it has none.
fn scheme(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    let valid = !scheme.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then(|| scheme.to_ascii_lowercase())
}

/// Hosts HTTPS-first leaves alone (spec 05 §5 rule 1).
fn exempt_from_upgrade(url: &Url) -> bool {
    match url.host() {
        None => true,
        Some(Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost"
                || [".localhost", ".local", ".test", ".internal"]
                    .iter()
                    .any(|s| d.ends_with(s))
        }
        Some(Host::Ipv4(ip)) => {
            ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified()
        }
        Some(Host::Ipv6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

/// Decides one navigation (spec 02 §3).
pub fn decide(req: &NavRequest, ctx: &PolicyContext<'_>) -> NavDecision {
    if let NavPhase::Response {
        attachment,
        can_show,
        ..
    } = &req.phase
    {
        // Rule 7; rules 1 to 6 ran when the request started.
        return if *attachment || !can_show {
            NavDecision::Download
        } else {
            NavDecision::Allow
        };
    }
    let Some(dest) = scheme(&req.url) else {
        return NavDecision::Cancel;
    };
    let source = req.source_url.as_deref().and_then(scheme);
    let from_page = !req.initiated_by_mado;
    // 1. mado: only from mado: pages, or from Mado itself.
    if dest == "mado" && from_page && source.as_deref() != Some("mado") {
        return NavDecision::Cancel;
    }
    // 2. Top-level javascript: and data: only when Mado starts them.
    if (dest == "javascript" || dest == "data") && req.is_main_frame && from_page {
        return NavDecision::Cancel;
    }
    if dest == "javascript" {
        // Sub-frame javascript: URLs run inside the page; nothing to load.
        return NavDecision::Allow;
    }
    // 3. Another app's scheme: the app link flow.
    if !INTERNAL_SCHEMES.contains(&dest.as_str()) {
        return NavDecision::OpenExternal {
            url: req.url.clone(),
            reason: ExternalReason::AppLink,
        };
    }
    // 4. file: only from file: pages, or from Mado itself.
    if dest == "file" && from_page && source.as_deref() != Some("file") {
        return NavDecision::Cancel;
    }
    let parsed = Url::parse(&req.url).ok();
    // 5. "Open in Safari" for user-initiated top-level navigations.
    if req.is_main_frame
        && (req.user_gesture || req.initiated_by_mado)
        && let Some(host) = parsed.as_ref().and_then(Url::host_str)
        && (ctx.opens_externally)(host)
    {
        return NavDecision::OpenExternal {
            url: req.url.clone(),
            reason: ExternalReason::EngineRule,
        };
    }
    // 6. HTTPS-first.
    if req.is_main_frame
        && dest == "http"
        && ctx.https_first != HttpsFirst::Off
        && let Some(mut url) = parsed
        && !exempt_from_upgrade(&url)
        && !(ctx.http_fallback)(&url)
        && url.set_scheme("https").is_ok()
    {
        if url.port() == Some(80) {
            let _ = url.set_port(None);
        }
        return NavDecision::Redirect { url: url.into() };
    }
    // 8. Allow.
    NavDecision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use mado_engine::NavKind;
    use mado_engine::tab::Modifiers;

    #[derive(Clone, Copy)]
    enum Src {
        Mado,
        Page(&'static str),
    }

    fn request(url: &str, from: Src, main: bool, gesture: bool) -> NavRequest {
        let (source_url, initiated_by_mado) = match from {
            Src::Mado => (None, true),
            Src::Page(p) => (Some(p.to_owned()), false),
        };
        NavRequest {
            url: url.to_owned(),
            source_url,
            is_main_frame: main,
            user_gesture: gesture,
            initiated_by_mado,
            kind: NavKind::Link,
            phase: NavPhase::Action {
                modifiers: Modifiers::default(),
                method: "GET".into(),
            },
        }
    }

    fn ctx(mode: HttpsFirst) -> PolicyContext<'static> {
        PolicyContext {
            https_first: mode,
            opens_externally: &|host| host == "netflix-only.example",
            http_fallback: &|url| url.host_str() == Some("continued.example"),
        }
    }

    fn redirect(url: &str) -> NavDecision {
        NavDecision::Redirect { url: url.into() }
    }

    fn external(url: &str, reason: ExternalReason) -> NavDecision {
        NavDecision::OpenExternal {
            url: url.into(),
            reason,
        }
    }

    #[test]
    fn the_decision_table() {
        use ExternalReason::*;
        use NavDecision::{Allow, Cancel};
        use Src::*;
        let page = Page("https://site.example/");
        let rows: Vec<(&str, Src, bool, bool, NavDecision)> = vec![
            // 1. mado: from a normal page is cancelled; from Mado or mado: it loads.
            ("mado://history", page, true, true, Cancel),
            ("mado://settings", page, false, false, Cancel),
            ("MADO://history", page, true, true, Cancel),
            ("mado://history", Mado, true, false, Allow),
            ("mado://history", Page("mado://newtab"), true, true, Allow),
            // 2. Top-level javascript: and data: from pages are cancelled.
            ("javascript:alert(1)", page, true, true, Cancel),
            ("data:text/html,<p>x", page, true, true, Cancel),
            ("data:text/html,<p>x", Mado, true, false, Allow),
            ("data:text/html,<p>x", page, false, false, Allow),
            ("javascript:void(0)", page, false, false, Allow),
            // 3. External schemes go to the app link flow, never a navigation.
            (
                "zoommtg://zoom.us/join?confno=1",
                page,
                true,
                true,
                external("zoommtg://zoom.us/join?confno=1", AppLink),
            ),
            (
                "mailto:a@b.example",
                page,
                true,
                true,
                external("mailto:a@b.example", AppLink),
            ),
            (
                "tel:+1555",
                page,
                false,
                false,
                external("tel:+1555", AppLink),
            ),
            ("chrome-extension://abc/x.html", page, true, true, Allow),
            ("about:blank", page, false, false, Allow),
            ("blob:https://site.example/1", page, true, true, Allow),
            // 4. file: only from file: pages or Mado.
            ("file:///etc/passwd", page, true, true, Cancel),
            (
                "file:///Users/a/b.html",
                Page("file:///Users/a/"),
                true,
                true,
                Allow,
            ),
            ("file:///Users/a/b.html", Mado, true, false, Allow),
            // 5. "Open in Safari" rules: top-level and user-initiated only.
            (
                "https://netflix-only.example/watch",
                page,
                true,
                true,
                external("https://netflix-only.example/watch", EngineRule),
            ),
            (
                "https://netflix-only.example/watch",
                Mado,
                true,
                false,
                external("https://netflix-only.example/watch", EngineRule),
            ),
            (
                "https://netflix-only.example/watch",
                page,
                true,
                false,
                Allow,
            ),
            (
                "https://netflix-only.example/frame",
                page,
                false,
                true,
                Allow,
            ),
            // 6. HTTPS-first upgrades public hosts on top-level navigations.
            (
                "http://news.example/a?b=1#c",
                page,
                true,
                true,
                redirect("https://news.example/a?b=1#c"),
            ),
            (
                "http://news.example:80/",
                Mado,
                true,
                false,
                redirect("https://news.example/"),
            ),
            (
                "http://news.example:8080/",
                Mado,
                true,
                false,
                redirect("https://news.example:8080/"),
            ),
            (
                "http://8.8.8.8/",
                Mado,
                true,
                false,
                redirect("https://8.8.8.8/"),
            ),
            ("http://news.example/", page, false, false, Allow),
            ("http://localhost:3000/", Mado, true, false, Allow),
            ("http://app.localhost/", Mado, true, false, Allow),
            ("http://printer.local/", Mado, true, false, Allow),
            ("http://site.test/", Mado, true, false, Allow),
            ("http://corp.internal/", Mado, true, false, Allow),
            ("http://192.168.1.1/", Mado, true, false, Allow),
            ("http://10.0.0.2/", Mado, true, false, Allow),
            ("http://127.0.0.1/", Mado, true, false, Allow),
            ("http://[::1]/", Mado, true, false, Allow),
            ("http://[fd00::1]/", Mado, true, false, Allow),
            ("http://continued.example/", Mado, true, false, Allow),
            // 8. Everything else loads.
            ("https://news.example/", page, true, true, Allow),
            // Not a URL at all.
            ("no scheme here", page, true, true, Cancel),
            ("", Mado, true, false, Cancel),
        ];
        for (url, from, main, gesture, want) in rows {
            let got = decide(
                &request(url, from, main, gesture),
                &ctx(HttpsFirst::Upgrade),
            );
            assert_eq!(got, want, "{url} main={main} gesture={gesture}");
        }
    }

    #[test]
    fn https_first_off_leaves_http_alone_and_strict_still_upgrades() {
        let req = request("http://news.example/", Src::Mado, true, false);
        assert_eq!(decide(&req, &ctx(HttpsFirst::Off)), NavDecision::Allow);
        assert_eq!(
            decide(&req, &ctx(HttpsFirst::Strict)),
            redirect("https://news.example/")
        );
        assert_eq!(HttpsFirst::from_setting("strict"), HttpsFirst::Strict);
        assert_eq!(HttpsFirst::from_setting("off"), HttpsFirst::Off);
        assert_eq!(HttpsFirst::from_setting("upgrade"), HttpsFirst::Upgrade);
    }

    #[test]
    fn responses_download_when_attached_or_unshowable() {
        let response = |attachment, can_show| NavRequest {
            phase: NavPhase::Response {
                mime: "application/zip".into(),
                attachment,
                can_show,
            },
            ..request("https://site.example/f.zip", Src::Mado, true, false)
        };
        let c = ctx(HttpsFirst::Upgrade);
        assert_eq!(decide(&response(true, true), &c), NavDecision::Download);
        assert_eq!(decide(&response(false, false), &c), NavDecision::Download);
        assert_eq!(decide(&response(false, true), &c), NavDecision::Allow);
    }
}
