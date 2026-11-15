//! Each reader on its fixture home (spec 12 §7 check 6).

use std::path::Path;

use super::*;
use crate::fixtures::home;

const T1: i64 = 1_793_491_200_000;
const T2: i64 = 1_793_577_600_000;
const OLD: i64 = 1_780_000_000_000;
const ALL: [Kind; 5] = [
    Kind::Bookmarks,
    Kind::History,
    Kind::SearchEngines,
    Kind::Tabs,
    Kind::Extensions,
];

fn urls(b: &[Bookmark]) -> Vec<String> {
    b.iter()
        .flat_map(|n| {
            let mut v: Vec<String> = n.url.iter().cloned().collect();
            v.extend(urls(&n.children));
            v
        })
        .collect()
}

fn source(home: &Path, browser: Browser) -> Source {
    detect(home)
        .into_iter()
        .find(|s| s.browser == browser)
        .expect("detected")
}

#[test]
fn times_convert_from_each_epoch() {
    assert_eq!(from_chromium(13_437_964_800_000_000), T1);
    assert_eq!(from_firefox(T1 * 1000), T1);
    assert_eq!(from_safari(815_184_000.0), T1);
    assert_eq!(from_safari(0.0), 978_307_200_000);
}

#[test]
fn detection_finds_each_browser_and_only_existing_profiles() {
    let chrome = home("chrome");
    let found = detect(chrome.path());
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        (
            found[0].browser,
            found[0].profile.as_str(),
            found[0].name.as_str()
        ),
        (Browser::Chrome, "Default", "Personal")
    );
    let firefox = home("firefox");
    let found = detect(firefox.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "default-release");
    assert_eq!(detect(home("safari").path())[0].browser, Browser::Safari);
    assert_eq!(detect(home("arc").path())[0].browser, Browser::Arc);
    assert!(detect(tempfile::tempdir().expect("dir").path()).is_empty());
}

#[test]
fn chrome_reads_everything() {
    let h = home("chrome");
    let s = source(h.path(), Browser::Chrome);
    let all = read(&s, &ALL, None, h.path()).expect("read");
    assert_eq!(
        urls(&all.bar),
        [
            "https://www.rust-lang.org/",
            "https://news.ycombinator.com/"
        ]
    );
    assert_eq!(all.bar[1].title, "News");
    assert_eq!(urls(&all.other), ["https://example.com/other"]);
    assert_eq!(all.visits.len(), 4);
    assert_eq!(all.visits[0].at, OLD);
    assert_eq!(all.visits.last().map(|v| v.at), Some(T2));
    let names: Vec<&str> = all.engines.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["Google", "Wikipedia"],
        "chrome:// templates are skipped"
    );
    assert_eq!(all.default_engine.as_deref(), Some("Wikipedia"));
    assert_eq!(all.windows.len(), 2, "the closed window is gone");
    let first = &all.windows[0];
    assert_eq!(first.len(), 2, "the closed tab is gone");
    assert!(first[0].pinned && first[0].url == "https://mail.example/");
    assert_eq!(first[1].url, "https://b.example/");
    assert_eq!(first[1].title, "B — ünïcode");
    assert!(first[1].group.is_some());
    assert_eq!(all.windows[1][0].url, "https://second.example/");
    assert_eq!(
        all.extensions,
        [Extension {
            id: "cjpalhdlnbpafiamejdnhcphjbkeiagm".into(),
            name: "uBlock Origin".into()
        }]
    );
    let recent = read(&s, &[Kind::History], Some(T1), h.path()).expect("read");
    assert_eq!(recent.visits.len(), 3);
    assert!(recent.windows.is_empty() && recent.bar.is_empty());
}

#[test]
fn firefox_reads_everything() {
    let h = home("firefox");
    let s = source(h.path(), Browser::Firefox);
    let all = read(&s, &ALL, None, h.path()).expect("read");
    assert_eq!(
        urls(&all.bar),
        ["https://www.mozilla.org/", "https://developer.mozilla.org/"],
        "place: queries skipped"
    );
    assert_eq!(all.bar[1].title, "Docs");
    assert_eq!(urls(&all.other), ["https://developer.mozilla.org/"]);
    assert_eq!(all.visits.len(), 3);
    assert_eq!(
        read(&s, &[Kind::History], Some(T1), h.path())
            .expect("read")
            .visits
            .len(),
        2
    );
    assert_eq!(
        all.engines,
        [
            Engine {
                name: "Google".into(),
                keyword: String::new(),
                search_url: "https://www.google.com/search?q={searchTerms}".into()
            },
            Engine {
                name: "Marginalia".into(),
                keyword: "@m".into(),
                search_url: "https://search.marginalia.nu/search?query={searchTerms}".into()
            },
        ]
    );
    assert_eq!(all.default_engine.as_deref(), Some("Marginalia"));
    assert_eq!(all.windows.len(), 1);
    assert!(all.windows[0][0].pinned);
    assert_eq!(all.windows[0][1].url, "https://two.example/");
    assert_eq!(all.extensions.len(), 1);
}

#[test]
fn safari_reads_bookmarks_history_and_the_default_engine() {
    let h = home("safari");
    let s = source(h.path(), Browser::Safari);
    let all = read(&s, &ALL, None, h.path()).expect("read");
    assert_eq!(
        urls(&all.bar),
        ["https://www.apple.com/", "https://webkit.org/"]
    );
    assert_eq!(all.bar[0].title, "Apple");
    assert_eq!(
        urls(&all.other),
        ["https://www.swift.org/"],
        "Reading List skipped"
    );
    assert_eq!(all.visits.len(), 3);
    assert_eq!(all.visits.iter().map(|v| v.at).max(), Some(T2));
    assert_eq!(all.default_engine.as_deref(), Some("DuckDuckGo"));
    assert!(all.windows.is_empty());
}

#[test]
fn safari_without_full_disk_access_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let h = home("safari");
    let s = source(h.path(), Browser::Safari);
    let plist = h.path().join("Library/Safari/Bookmarks.plist");
    std::fs::set_permissions(&plist, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    let err = read(&s, &[Kind::Bookmarks], None, h.path()).expect_err("refused");
    assert!(matches!(err, ImportError::FullDiskAccess), "{err}");
}

#[test]
fn arc_reads_favorites_and_spaces() {
    let h = home("arc");
    let s = source(h.path(), Browser::Arc);
    let all = read(&s, &ALL, None, h.path()).expect("read");
    assert_eq!(urls(&all.other), ["https://arc.net/"]);
    assert_eq!(all.windows.len(), 1);
    assert_eq!(all.windows[0][0].url, "https://calendar.example/");
    assert!(all.windows[0][0].pinned);
    assert_eq!(all.spaces.len(), 1, "an empty space is skipped");
    assert_eq!(all.spaces[0].title, "Work");
    let tabs: Vec<(&str, &str)> = all.spaces[0]
        .tabs
        .iter()
        .map(|t| (t.url.as_str(), t.title.as_str()))
        .collect();
    assert_eq!(
        tabs,
        [
            ("https://jira.example/", "Jira"),
            ("https://spec.example/", "Spec")
        ]
    );
}

#[test]
fn malformed_files_are_errors_not_panics() {
    assert!(parse_snss(b"nope").is_err());
    assert_eq!(
        parse_snss(b"SNSS\x03\0\0\0\xff\xff\x06").expect("truncated"),
        Vec::<Vec<Tab>>::new()
    );
    let dir = tempfile::tempdir().expect("dir");
    let bad = dir.path().join("x.mozlz4");
    std::fs::write(&bad, b"mozLz40\0\xff\xff\xff\x7fjunk").expect("write");
    assert!(firefox::mozlz4(&bad).is_err());
}
