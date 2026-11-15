//! What the address bar shows for a URL (spec 05 §4).

use crate::idn::{TopDomains, host_for_display};

/// The address bar text for a page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Display {
    pub text: String,
    /// The registrable domain within `text`, in UTF-16 code units (the UI's
    /// string offsets); empty when nothing is emphasised.
    pub emphasis: (u32, u32),
    /// A label stays in punycode because it failed the spoof checks.
    pub punycode: bool,
    /// The URL carries `user:pass@`, which is never shown.
    pub userinfo: bool,
}

fn utf16_len(s: &str) -> u32 {
    u32::try_from(s.encode_utf16().count()).unwrap_or(u32::MAX)
}

/// `full` is `addressBar.show = fullUrl`; otherwise only the registrable
/// domain shows.
pub fn display(raw: &str, full: bool, top: &TopDomains) -> Display {
    let Ok(url) = url::Url::parse(raw) else {
        return Display {
            text: raw.to_owned(),
            ..Display::default()
        };
    };
    let userinfo = !url.username().is_empty() || url.password().is_some();
    match url.scheme() {
        "data" | "blob" | "javascript" => {
            return Display {
                text: format!("{}:", url.scheme()),
                userinfo,
                ..Display::default()
            };
        }
        "about" => {
            return Display {
                text: url.as_str().to_owned(),
                ..Display::default()
            };
        }
        "file" => {
            let path = url.to_file_path().map_or_else(
                |()| url.path().to_owned(),
                |p| p.to_string_lossy().into_owned(),
            );
            return Display {
                text: path,
                ..Display::default()
            };
        }
        _ => {}
    }
    let Some(host) = url.host_str() else {
        return Display {
            text: url.as_str().to_owned(),
            userinfo,
            ..Display::default()
        };
    };
    let (shown_host, punycode) = host_for_display(host, top);
    // The registrable domain, as the same number of trailing labels.
    let registrable_labels = psl::domain(host.as_bytes())
        .map(|d| d.as_bytes().split(|b| *b == b'.').count())
        .unwrap_or(0);
    let labels: Vec<&str> = shown_host.split('.').collect();
    let registrable = if registrable_labels > 0 && registrable_labels <= labels.len() {
        labels[labels.len() - registrable_labels..].join(".")
    } else {
        shown_host.clone()
    };
    if !full && url.scheme() != "mado" {
        let len = utf16_len(&registrable);
        return Display {
            text: registrable,
            emphasis: (0, len),
            punycode,
            userinfo,
        };
    }
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    let prefix = format!("{}://", url.scheme());
    let tail = &url.as_str()[url[..url::Position::AfterPort].len()..];
    let text = format!("{prefix}{shown_host}{port}{tail}");
    let end = utf16_len(&prefix) + utf16_len(&shown_host);
    Display {
        text,
        emphasis: (end - utf16_len(&registrable), end),
        punycode,
        userinfo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top() -> TopDomains {
        TopDomains::from_list("google.com\nexample.com")
    }

    #[test]
    fn site_mode_shows_the_registrable_domain() {
        let d = display("https://mail.google.com/mail/u/0", false, &top());
        assert_eq!(d.text, "google.com");
        assert_eq!(d.emphasis, (0, 10));
    }

    #[test]
    fn full_mode_emphasises_the_registrable_domain_and_drops_userinfo() {
        let d = display("https://user:pw@www.bbc.co.uk:8443/news?x#y", true, &top());
        assert_eq!(d.text, "https://www.bbc.co.uk:8443/news?x#y");
        assert_eq!(d.emphasis, (12, 21));
        assert!(d.userinfo);
    }

    #[test]
    fn idn_shows_unicode_only_when_safe() {
        let safe = display("https://xn--bcher-kva.de/", false, &top());
        assert_eq!((safe.text.as_str(), safe.punycode), ("bücher.de", false));
        // "gооgle.com" with Cyrillic о: a lookalike of google.com.
        let spoof = display("https://xn--ggle-55da.com/", false, &top());
        assert_eq!(
            (spoof.text.as_str(), spoof.punycode),
            ("xn--ggle-55da.com", true)
        );
    }

    #[test]
    fn data_about_and_files_show_no_host() {
        assert_eq!(display("data:text/html,hi", true, &top()).text, "data:");
        assert_eq!(display("about:blank", false, &top()).text, "about:blank");
        assert_eq!(
            display("file:///Users/a%20b/x.txt", false, &top()).text,
            "/Users/a b/x.txt"
        );
        assert_eq!(
            display("mado://settings", false, &top()).text,
            "mado://settings"
        );
    }
}
