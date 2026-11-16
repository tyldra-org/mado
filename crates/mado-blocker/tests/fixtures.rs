//! The blocker fixture pages of spec 10 §9 check 1 on every CI run, without
//! WebKit: the pages in `tests/pages/blocker/` make the requests below, and
//! both adblock-rust's engine and the WebKit content rule lists Mado
//! compiles (evaluated here as WebKit does) give each its expected verdict.
//! `tests/e2e/blocker.test.ts` and `engine.test.ts` load the same pages
//! in real WebKit on macOS.

use std::collections::HashMap;

use regex::{Regex, RegexBuilder};
use serde_json::Value;

const ORIGIN: &str = "http://pages.mado.test";

/// One WebKit content rule list, evaluated as WebKit does: the rules in
/// order, `ignore-previous-rules` dropping what matched before it.
struct RuleList {
    rules: Vec<Value>,
    compiled: HashMap<(String, bool), Option<Regex>>,
}

fn host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn domain_matches(list: &Value, host: &str) -> bool {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|d| match d.strip_prefix('*') {
            Some(d) => host == d || host.ends_with(&format!(".{d}")),
            None => host == d,
        })
}

/// The longest run of characters any match must contain, to skip compiling
/// most of the list's patterns.
fn literal(filter: &str) -> Option<String> {
    if filter.contains('|') || filter.contains('(') {
        return None;
    }
    let mut runs = vec![String::new()];
    let mut chars = filter.chars().peekable();
    while let Some(c) = chars.next() {
        let lit = match c {
            '\\' => chars.next(),
            '[' => {
                for n in chars.by_ref() {
                    if n == ']' {
                        break;
                    }
                }
                None
            }
            '.' | '^' | '$' | '+' | '*' | '?' | '{' | '}' | ']' => None,
            c => Some(c),
        };
        match lit {
            Some(l) if !matches!(chars.peek(), Some('?' | '*' | '{')) => {
                if let Some(r) = runs.last_mut() {
                    r.push(l);
                }
            }
            _ => runs.push(String::new()),
        }
    }
    runs.into_iter()
        .max_by_key(String::len)
        .filter(|r| r.len() >= 3)
}

impl RuleList {
    fn new(json: &str) -> Self {
        Self {
            rules: serde_json::from_str(json).expect("a rule list"),
            compiled: HashMap::new(),
        }
    }

    fn url_matches(&mut self, trigger: &Value, url: &str) -> bool {
        let filter = trigger["url-filter"].as_str().unwrap_or(".*");
        let case = trigger["url-filter-is-case-sensitive"]
            .as_bool()
            .unwrap_or(false);
        if let Some(l) = literal(filter) {
            let found = if case {
                url.contains(&l)
            } else {
                url.to_lowercase().contains(&l.to_lowercase())
            };
            if !found {
                return false;
            }
        }
        self.compiled
            .entry((filter.to_owned(), case))
            .or_insert_with(|| {
                RegexBuilder::new(filter)
                    .case_insensitive(!case)
                    .build()
                    .ok()
            })
            .as_ref()
            .is_some_and(|r| r.is_match(url))
    }

    /// Whether WebKit blocks `url`, a `kind` load of the page at `page`.
    fn blocks(&mut self, url: &str, page: &str, kind: &str) -> bool {
        let page_host = host(page);
        let third_party = host(url) != page_host;
        let mut blocked = false;
        for i in 0..self.rules.len() {
            let rule = self.rules[i].clone();
            let t = &rule["trigger"];
            let action = rule["action"]["type"].as_str().unwrap_or_default();
            if !matches!(action, "block" | "ignore-previous-rules") {
                continue;
            }
            if let Some(types) = t["resource-type"].as_array()
                && !types.iter().any(|x| x == kind)
            {
                continue;
            }
            if let Some(load) = t["load-type"].as_array() {
                let want = if third_party {
                    "third-party"
                } else {
                    "first-party"
                };
                if !load.iter().any(|x| x == want) {
                    continue;
                }
            }
            if !t["if-domain"].is_null() && !domain_matches(&t["if-domain"], &page_host) {
                continue;
            }
            if domain_matches(&t["unless-domain"], &page_host) || !self.url_matches(t, url) {
                continue;
            }
            blocked = action == "block";
        }
        blocked
    }
}

/// Several lists, as a tab's controller holds them: blocked when any list
/// blocks.
fn webkit(lists: &mut [RuleList], url: &str, page: &str, kind: &str) -> bool {
    lists.iter_mut().any(|l| l.blocks(url, page, kind))
}

fn page(path: &str) -> (String, String) {
    let file = format!("{}/../../tests/pages{path}", env!("CARGO_MANIFEST_DIR"));
    let html = std::fs::read_to_string(&file).expect("the fixture page");
    (format!("{ORIGIN}{path}"), html)
}

/// `network.html` with the bundled EasyList: the ad script is blocked, the
/// app's script loads, and `.ADBox` is hidden by a generic rule.
#[test]
fn the_network_and_generic_hiding_fixture() {
    let (url, html) = page("/blocker/network.html");
    let (ad, app) = ("/blocker/ads/ad_header.js", "/blocker/app.js");
    assert!(html.contains(ad) && html.contains(app) && html.contains("ADBox"));
    let easylist = mado_blocker::bundled_easylist();

    let engine = mado_blocker::custom_engine(std::slice::from_ref(&easylist), Vec::new());
    assert!(mado_blocker::blocks(
        &engine,
        &format!("{ORIGIN}{ad}"),
        &url,
        "script"
    ));
    assert!(!mado_blocker::blocks(
        &engine,
        &format!("{ORIGIN}{app}"),
        &url,
        "script"
    ));
    let hidden = engine.hidden_class_id_selectors(["ADBox"], [] as [&str; 0], &Default::default());
    assert!(hidden.iter().any(|s| s == ".ADBox"), "{hidden:?}");

    let (rules, _) = mado_blocker::rules::convert([easylist]);
    let mut lists: Vec<RuleList> =
        mado_blocker::rules::shard(rules, mado_blocker::rules::MAX_RULES)
            .iter()
            .map(|s| RuleList::new(s))
            .collect();
    assert!(webkit(&mut lists, &format!("{ORIGIN}{ad}"), &url, "script"));
    assert!(!webkit(
        &mut lists,
        &format!("{ORIGIN}{app}"),
        &url,
        "script"
    ));
    // The page itself loads. Generic hiding (`.ADBox`) reaches it through
    // the engine's selectors above, which the cosmetic page script applies:
    // the converted list ends with an `ignore-previous-rules` for
    // first-party documents.
    assert!(!webkit(&mut lists, &url, &url, "document"));
}

/// `exception.html` with custom filters: a block and its exception in the
/// profile's own list.
#[test]
fn the_exception_fixture() {
    let (url, html) = page("/blocker/exception.html");
    let (blocked, kept) = (
        "/blocker/exception/blocked.js",
        "/blocker/exception/kept.js",
    );
    assert!(html.contains(blocked) && html.contains(kept));
    let custom = [
        "||pages.mado.test/blocker/exception/".to_owned(),
        "@@||pages.mado.test/blocker/exception/kept.js".to_owned(),
    ];
    let engine = mado_blocker::custom_engine(&custom, Vec::new());
    assert!(mado_blocker::blocks(
        &engine,
        &format!("{ORIGIN}{blocked}"),
        &url,
        "script"
    ));
    assert!(!mado_blocker::blocks(
        &engine,
        &format!("{ORIGIN}{kept}"),
        &url,
        "script"
    ));

    let mut lists = [RuleList::new(
        &mado_blocker::rules::profile_rules(&custom, &[]).expect("a profile list"),
    )];
    assert!(webkit(
        &mut lists,
        &format!("{ORIGIN}{blocked}"),
        &url,
        "script"
    ));
    assert!(!webkit(
        &mut lists,
        &format!("{ORIGIN}{kept}"),
        &url,
        "script"
    ));
}

/// `cosmetic.html`: the procedural filter and the scriptlet reach the page
/// through the engine's cosmetic resources (WebKit has no rules for them).
#[test]
fn the_cosmetic_and_scriptlet_fixture() {
    let (url, html) = page("/blocker/cosmetic.html");
    assert!(html.contains("Sponsored") && html.contains("madoScriptlet"));
    let resource: mado_blocker::Resource = serde_json::from_value(serde_json::json!({
        "name": "mado-test-mark.js", "aliases": ["mado-test-mark"], "kind": "template",
        "content": base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            "(function(){ window.madoScriptlet = '{{1}}'; })();",
        ),
    }))
    .expect("resource");
    let engine = mado_blocker::custom_engine(
        &[
            "pages.mado.test##.card:has-text(Sponsored)".to_owned(),
            "pages.mado.test##+js(mado-test-mark, ok)".to_owned(),
            "||pages.mado.test/blocker/exception/".to_owned(),
            "@@||pages.mado.test/blocker/exception/kept.js".to_owned(),
        ],
        vec![resource],
    );
    let r = engine.url_cosmetic_resources(&url);
    assert!(
        r.procedural_actions
            .iter()
            .any(|a| a.contains("has-text") && a.contains("Sponsored")),
        "{:?}",
        r.procedural_actions
    );
    assert!(
        r.injected_script.contains("madoScriptlet"),
        "{}",
        r.injected_script
    );
}

#[test]
fn literals_never_skip_a_match() {
    for (filter, url) in [
        (
            r"^[^:]+:(//)?([^/]+\.)?example\.com[/:]",
            "https://ads.example.com/x",
        ),
        (
            r"/ad_header\.js",
            "http://pages.mado.test/blocker/ads/ad_header.js",
        ),
        (r"https?://x\.com/a?b", "https://x.com/b"),
    ] {
        if let Some(l) = literal(filter) {
            assert!(url.contains(&l), "{filter}: {l}");
        }
        assert!(Regex::new(filter).expect("regex").is_match(url), "{filter}");
    }
}
