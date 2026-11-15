//! Address bar suggestions (spec 05 §2): every local provider scored on
//! the spec's scale, merged, and the inline completion.

use serde::{Deserialize, Serialize};

use crate::Input;

/// Where a row came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RowKind {
    Typed,
    Calculator,
    Tab,
    History,
    Bookmark,
    SearchSuggestion,
    Keyword,
    Clipboard,
    PageText,
}

/// One row of the suggestion list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub kind: RowKind,
    /// What `nav.open` gets when the row is chosen.
    pub input: String,
    pub title: String,
    pub url: Option<String>,
    /// "Switch to tab" rows.
    pub tab_id: Option<String>,
    pub score: u32,
    /// Shift+Delete removes it (history rows).
    pub removable: bool,
}

/// A visited URL from the history provider.
#[derive(Debug, Clone)]
pub struct Visited {
    pub url: String,
    pub title: String,
    pub frecency: f64,
    pub typed_count: i64,
}

/// What the providers found, gathered by the core.
#[derive(Debug, Default)]
pub struct Sources<'a> {
    pub tabs: &'a [(String, String, String)],
    pub history: &'a [Visited],
    pub bookmarks: &'a [(String, String)],
    /// Engine keywords with the engine's name.
    pub keywords: &'a [(String, String)],
    /// A link copied in the last 3 minutes.
    pub clipboard: Option<&'a str>,
    /// Page text matches: URL and title.
    pub page_text: &'a [(String, String)],
    /// Private windows: only tabs, bookmarks, typed text, the calculator and
    /// remote suggestions.
    pub private: bool,
}

/// At most this many rows (spec 05 §2).
pub const MAX_ROWS: usize = 8;

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn strip_www(s: &str) -> &str {
    s.strip_prefix("www.").unwrap_or(s)
}

/// The URL without scheme and `www.`, as people type it.
fn typed_form(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    strip_www(rest).to_lowercase()
}

fn words_in(words: &[String], haystack: &str) -> bool {
    let h = haystack.to_lowercase();
    !words.is_empty() && words.iter().all(|w| h.contains(w.as_str()))
}

/// Ranks the local rows for `raw`, classified as `input`.
pub fn rank(raw: &str, input: &Input, src: &Sources<'_>) -> Vec<Row> {
    let text = raw.trim();
    let lower = text.to_lowercase();
    let words: Vec<String> = lower.split_whitespace().map(str::to_owned).collect();
    let mut rows: Vec<Row> = Vec::new();
    let row = |kind, input: String, title: String, url: Option<String>, score| Row {
        kind,
        input,
        title,
        url,
        tab_id: None,
        score,
        removable: kind == RowKind::History,
    };
    match input {
        Input::Url { url } => rows.push(row(
            RowKind::Typed,
            text.into(),
            url.clone(),
            Some(url.clone()),
            1300,
        )),
        Input::FilePath { url, .. } => rows.push(row(
            RowKind::Typed,
            text.into(),
            url.clone(),
            Some(url.clone()),
            1300,
        )),
        Input::Search { terms } => {
            rows.push(row(RowKind::Typed, text.into(), terms.clone(), None, 1150))
        }
        Input::Calculation { result, .. } => {
            rows.push(row(
                RowKind::Calculator,
                text.into(),
                result.clone(),
                None,
                1500,
            ));
            rows.push(row(RowKind::Typed, text.into(), text.into(), None, 1150));
        }
        Input::Keyword { keyword, query } => {
            if let Some((_, name)) = src
                .keywords
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(keyword))
            {
                rows.push(row(
                    RowKind::Keyword,
                    text.into(),
                    format!("{name}: {query}"),
                    None,
                    1250,
                ));
            }
        }
        Input::Refused { .. } => {}
    }
    if text.is_empty() {
        if let Some(link) = src.clipboard {
            rows.push(row(
                RowKind::Clipboard,
                link.into(),
                link.into(),
                Some(link.into()),
                1100,
            ));
        }
        return rows;
    }
    for (id, title, url) in src.tabs {
        let prefix =
            typed_form(url).starts_with(&lower) || title.to_lowercase().starts_with(&lower);
        let quality = if prefix {
            1.0
        } else if words_in(&words, title) || words_in(&words, url) {
            0.8
        } else {
            continue;
        };
        rows.push(Row {
            tab_id: Some(id.clone()),
            ..row(
                RowKind::Tab,
                url.clone(),
                title.clone(),
                Some(url.clone()),
                (1200.0 * quality) as u32,
            )
        });
    }
    for (title, url) in src.bookmarks {
        let score = if title.to_lowercase().starts_with(&lower) {
            1350
        } else if strip_www(&host_of(url)).starts_with(&lower) {
            1300
        } else if words_in(&words, title) || words_in(&words, url) {
            1150
        } else {
            continue;
        };
        rows.push(row(
            RowKind::Bookmark,
            url.clone(),
            title.clone(),
            Some(url.clone()),
            score,
        ));
    }
    if !src.private {
        for v in src.history {
            let host_prefix = strip_www(&host_of(&v.url)).starts_with(&lower);
            if !host_prefix && !words_in(&words, &v.title) && !words_in(&words, &v.url) {
                continue;
            }
            let score = 900.0
                + (v.frecency.clamp(0.0, 2000.0) / 2000.0) * 400.0
                + if host_prefix { 100.0 } else { 0.0 };
            rows.push(row(
                RowKind::History,
                v.url.clone(),
                v.title.clone(),
                Some(v.url.clone()),
                (score as u32).min(1400),
            ));
        }
        if words.len() >= 2 {
            for (url, title) in src.page_text.iter().take(2) {
                rows.push(row(
                    RowKind::PageText,
                    url.clone(),
                    title.clone(),
                    Some(url.clone()),
                    700,
                ));
            }
        }
    }
    merge(rows)
}

/// Adds remote search suggestions (scored 900 down to 600 by rank) to the
/// local rows.
pub fn with_remote(local: Vec<Row>, phrases: &[String]) -> Vec<Row> {
    let mut rows = local;
    for (i, p) in phrases.iter().enumerate() {
        let score = 900u32
            .saturating_sub(50 * u32::try_from(i).unwrap_or(6))
            .max(600);
        rows.push(Row {
            kind: RowKind::SearchSuggestion,
            input: p.clone(),
            title: p.clone(),
            url: None,
            tab_id: None,
            score,
            removable: false,
        });
    }
    merge(rows)
}

/// Best first, one row per URL (and per search phrase), at most
/// [`MAX_ROWS`].
fn merge(mut rows: Vec<Row>) -> Vec<Row> {
    rows.sort_by_key(|r| std::cmp::Reverse(r.score));
    let mut seen = std::collections::HashSet::new();
    rows.retain(|r| {
        let key = match (&r.url, r.kind) {
            (Some(u), RowKind::Tab) => format!("tab {u}"),
            (Some(u), _) => typed_form(u),
            (None, _) => format!("text {}", r.title.to_lowercase()),
        };
        seen.insert(key)
    });
    rows.truncate(MAX_ROWS);
    rows
}

/// The inline completion after the cursor (spec 05 §2): the rest of the top
/// history or bookmark row's host when it starts with what was typed, then
/// the path when that URL was typed at least twice. `None` after a
/// deletion, with a space, or when nothing fits.
pub fn inline(raw: &str, rows: &[Row], history: &[Visited], deleted: bool) -> Option<String> {
    let typed = raw.to_lowercase();
    if deleted || typed.is_empty() || typed.contains(char::is_whitespace) {
        return None;
    }
    let top = rows.first()?;
    if !matches!(top.kind, RowKind::History | RowKind::Bookmark) {
        return None;
    }
    let url = top.url.as_deref()?;
    let host = strip_www(&host_of(url)).to_lowercase();
    if let Some(rest) = host.strip_prefix(&typed)
        && !rest.is_empty()
    {
        return Some(rest.to_owned());
    }
    let typed_twice = history.iter().any(|v| v.url == url && v.typed_count >= 2);
    let full = typed_form(url);
    let full = full.trim_end_matches('/');
    match full.strip_prefix(&typed) {
        Some(rest) if typed_twice && !rest.is_empty() => Some(rest.to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visited(url: &str, frecency: f64, typed: i64) -> Visited {
        Visited {
            url: url.into(),
            title: String::new(),
            frecency,
            typed_count: typed,
        }
    }

    #[test]
    fn rows_follow_the_spec_scores_and_merge_by_url() {
        let history = [
            visited("https://github.com/", 2000.0, 3),
            visited("https://gitlab.com/", 100.0, 0),
        ];
        let tabs = [(
            "t1".to_owned(),
            "GitHub".to_owned(),
            "https://github.com/".to_owned(),
        )];
        let src = Sources {
            tabs: &tabs,
            history: &history,
            ..Sources::default()
        };
        let rows = rank(
            "git",
            &Input::Search {
                terms: "git".into(),
            },
            &src,
        );
        let kinds: Vec<(RowKind, u32)> = rows.iter().map(|r| (r.kind, r.score)).collect();
        assert_eq!(
            kinds,
            vec![
                (RowKind::History, 1400),
                (RowKind::Tab, 1200),
                (RowKind::Typed, 1150),
                (RowKind::History, 1020)
            ]
        );
        assert_eq!(
            inline("git", &rows, &history, false).as_deref(),
            Some("hub.com")
        );
        assert_eq!(inline("git", &rows, &history, true), None);
        assert_eq!(inline("github.com/", &rows, &history, false), None);
    }

    #[test]
    fn private_windows_get_no_history_and_remote_rows_rank_by_order() {
        let history = [visited("https://github.com/", 2000.0, 3)];
        let src = Sources {
            history: &history,
            private: true,
            ..Sources::default()
        };
        let rows = rank(
            "git",
            &Input::Search {
                terms: "git".into(),
            },
            &src,
        );
        assert_eq!(rows.len(), 1);
        let rows = with_remote(rows, &["git clone".into(), "git rebase".into()]);
        let scores: Vec<u32> = rows.iter().map(|r| r.score).collect();
        assert_eq!(scores, vec![1150, 900, 850]);
    }
}
