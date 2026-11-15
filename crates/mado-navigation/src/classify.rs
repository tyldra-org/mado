//! The input classifier (spec 05 §1): what the text typed or pasted into
//! the address bar is, with Chrome's fixups and refusals.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Schemes the address bar navigates when typed (spec 05 §1).
pub const KNOWN_SCHEMES: &[&str] = &[
    "http",
    "https",
    "file",
    "mado",
    "about",
    "view-source",
    "data",
    "javascript",
    "ftp",
];

/// `@` scopes that search inside Mado instead of the web.
pub const SCOPES: &[&str] = &["@tabs", "@bookmarks", "@history"];

/// What the input is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "class",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Input {
    /// A URL to load, fixed up (`https://` added, spaces encoded).
    Url { url: String },
    /// A file on disk, as a `file://` URL.
    FilePath { path: String, url: String },
    /// A search engine keyword or an `@` scope, then the query.
    Keyword { keyword: String, query: String },
    /// Evaluated locally; never leaves the Mac.
    Calculation { expression: String, result: String },
    /// Goes to the default search engine.
    Search { terms: String },
    /// Never navigated.
    Refused { reason: Refusal },
}

/// Why an input is not navigated (spec 05 §1, refused input).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Refusal {
    /// Nothing but whitespace.
    Empty,
    /// `javascript:` typed or pasted into the address bar.
    Javascript,
    /// A `data:` URL that is not an image, a PDF or plain text.
    Data,
}

/// What the classifier needs to know about the profile and the Mac.
pub struct Context<'a> {
    /// Keywords of the active search engines.
    pub keywords: &'a [String],
    /// Whether a single-label host (`intranet`) was visited before.
    pub visited: &'a dyn Fn(&str) -> bool,
    /// Whether a path exists on disk.
    pub exists: &'a dyn Fn(&Path) -> bool,
    /// The home folder, for `~/`.
    pub home: Option<&'a Path>,
    /// Now, in the user's time zone, for time zone and date calculations.
    pub now: jiff::Zoned,
}

impl std::fmt::Debug for Context<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("keywords", &self.keywords)
            .field("home", &self.home)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// Classifies address bar input. `ctrl_enter` is ⌘Enter, which wraps a
/// bare word as `https://www.<word>.com`.
pub fn classify(raw: &str, ctx: &Context<'_>, ctrl_enter: bool) -> Input {
    let text = raw.trim();
    if text.is_empty() {
        return Input::Refused {
            reason: Refusal::Empty,
        };
    }
    if let Some(input) = with_scheme(text) {
        return input;
    }
    if let Some(input) = file_path(text, ctx) {
        return input;
    }
    if let Some((first, rest)) = text.split_once(char::is_whitespace) {
        let rest = rest.trim();
        let scope = SCOPES.iter().any(|s| s.eq_ignore_ascii_case(first));
        let keyword = ctx.keywords.iter().any(|k| k.eq_ignore_ascii_case(first));
        if (scope || keyword) && !rest.is_empty() {
            return Input::Keyword {
                keyword: first.to_lowercase(),
                query: rest.to_owned(),
            };
        }
    }
    if ctrl_enter && is_bare_word(text) {
        let url = format!("https://www.{}.com/", text.to_lowercase());
        return Input::Url { url };
    }
    if let Some(url) = host_like(text, ctx) {
        return Input::Url { url };
    }
    if let Some(result) = crate::calc::evaluate(text, &ctx.now) {
        return Input::Calculation {
            expression: text.to_owned(),
            result,
        };
    }
    Input::Search {
        terms: text.to_owned(),
    }
}

/// Input that starts with one of the known schemes.
fn with_scheme(text: &str) -> Option<Input> {
    let (scheme, rest) = text.split_once(':')?;
    let scheme = scheme.to_ascii_lowercase();
    if !KNOWN_SCHEMES.contains(&scheme.as_str()) {
        return None;
    }
    match scheme.as_str() {
        "javascript" => Some(Input::Refused {
            reason: Refusal::Javascript,
        }),
        "data" => {
            let mime = rest
                .split([',', ';'])
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            // SVG is an image that runs script, as HTML does.
            let allowed = mime.is_empty()
                || mime == "text/plain"
                || mime == "application/pdf"
                || (mime.starts_with("image/") && mime != "image/svg+xml");
            Some(if allowed {
                Input::Url {
                    url: text.to_owned(),
                }
            } else {
                Input::Refused {
                    reason: Refusal::Data,
                }
            })
        }
        "view-source" => {
            let inner = with_scheme(rest).or_else(|| {
                // view-source:example.com
                Some(Input::Url {
                    url: url::Url::parse(&format!("https://{}", encode_spaces(rest)))
                        .ok()?
                        .to_string(),
                })
            })?;
            match inner {
                Input::Url { url } => Some(Input::Url {
                    url: format!("view-source:{url}"),
                }),
                _ => None,
            }
        }
        _ => url::Url::parse(&encode_spaces(text))
            .ok()
            .map(|u| Input::Url { url: u.to_string() }),
    }
}

fn file_path(text: &str, ctx: &Context<'_>) -> Option<Input> {
    let path = if let Some(rest) = text.strip_prefix("~/") {
        ctx.home?.join(rest)
    } else if text.starts_with('/') {
        PathBuf::from(text)
    } else {
        return None;
    };
    if !(ctx.exists)(&path) {
        return None;
    }
    let url = url::Url::from_file_path(&path).ok()?.to_string();
    Some(Input::FilePath {
        path: path.to_string_lossy().into_owned(),
        url,
    })
}

fn is_bare_word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '-')
}

/// Percent-encodes whitespace inside a URL (spec 05 §1, fixups).
fn encode_spaces(text: &str) -> String {
    text.split(char::is_whitespace)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("%20")
}

/// `host[:port][/path]` without a scheme: the host has a public suffix, is
/// `localhost`, an IP literal or a visited single label; or it is dotted
/// and followed by a port or a path (`nas.local:8080`, `printer.lan/`).
fn host_like(text: &str, ctx: &Context<'_>) -> Option<String> {
    let end = text.find(['/', '?', '#']).unwrap_or(text.len());
    let (authority, rest) = text.split_at(end);
    if authority.is_empty() || authority.contains(char::is_whitespace) {
        return None;
    }
    // `name@example.com` without a scheme is an email address, not a URL.
    if authority.contains('@') {
        return None;
    }
    let (host, port) = split_port(authority)?;
    if let Some(port) = port
        && port.parse::<u16>().is_err()
    {
        return None;
    }
    let host_lower = host.to_lowercase();
    // A fully qualified name ends with a dot; the dot is not a label.
    let host = host.strip_suffix('.').unwrap_or(host);
    let qualified = !rest.is_empty() || port.is_some();
    let ok = if let Some(v6) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        v6.parse::<Ipv6Addr>().is_ok()
    } else if host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host.parse::<Ipv4Addr>().is_ok()
    {
        true
    } else if host.contains('.') {
        let labels_ok = host
            .split('.')
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_alphanumeric() || c == '-'));
        if !labels_ok || host.chars().all(|c| c.is_ascii_digit() || c == '.') {
            false
        } else {
            let ascii = idna_ascii(&host.to_lowercase())?;
            let known = psl::suffix(ascii.as_bytes()).is_some_and(|s| s.is_known())
                && psl::domain(ascii.as_bytes()).is_some();
            known || qualified
        }
    } else {
        host.chars().all(|c| c.is_alphanumeric() || c == '-') && (ctx.visited)(&host_lower)
    };
    if !ok {
        return None;
    }
    let url = url::Url::parse(&format!("https://{}", encode_spaces(text))).ok()?;
    url.host_str()?;
    Some(url.to_string())
}

fn split_port(host_port: &str) -> Option<(&str, Option<&str>)> {
    if host_port.starts_with('[') {
        let close = host_port.find(']')?;
        let (host, tail) = host_port.split_at(close + 1);
        return match tail.strip_prefix(':') {
            Some(port) => Some((host, Some(port))),
            None if tail.is_empty() => Some((host, None)),
            None => None,
        };
    }
    match host_port.rsplit_once(':') {
        Some((host, port)) => Some((host, Some(port))),
        None => Some((host_port, None)),
    }
}

fn idna_ascii(host: &str) -> Option<String> {
    url::Host::parse(host).ok().map(|h| h.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context<'static> {
        Context {
            keywords: &[],
            visited: &|_| false,
            exists: &|_| false,
            home: None,
            now: jiff::Timestamp::UNIX_EPOCH.to_zoned(jiff::tz::TimeZone::UTC),
        }
    }

    #[test]
    fn a_trailing_dot_and_case_still_make_a_url() {
        assert_eq!(
            classify("Example.COM.", &ctx(), false),
            Input::Url {
                url: "https://example.com./".into()
            }
        );
    }

    #[test]
    fn ports_out_of_range_are_searches() {
        assert!(matches!(
            classify("localhost:99999", &ctx(), false),
            Input::Search { .. }
        ));
    }
}
