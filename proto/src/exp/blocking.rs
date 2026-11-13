//! Experiments 6 and 7 (16 §5 rows 6 and 7, M0-T04), plus the engine numbers
//! both platforms share: build the adblock engine from the default lists
//! (10 §2, §3), time a request check against 10 §5's 50 µs budget, convert to
//! WebKit content rules, find WebKit's per-list rule limit and the cold
//! compile time of the full lists (10 §4), and check the private
//! blocked-action delegate (10 §4.7).

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use adblock::Engine;
use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use serde_json::{Value, json};

use super::Checks;
use super::sites::SITES;
use crate::fixtures::Fixtures;
use crate::macos;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};

/// The lists 10 §2 turns on by default.
const LISTS: [(&str, &str); 8] = [
    (
        "ubo-filters",
        "https://ublockorigin.github.io/uAssets/filters/filters.min.txt",
    ),
    (
        "ubo-privacy",
        "https://ublockorigin.github.io/uAssets/filters/privacy.min.txt",
    ),
    (
        "ubo-badware",
        "https://ublockorigin.github.io/uAssets/filters/badware.min.txt",
    ),
    (
        "ubo-quick",
        "https://ublockorigin.github.io/uAssets/filters/quick-fixes.min.txt",
    ),
    (
        "ubo-unbreak",
        "https://ublockorigin.github.io/uAssets/filters/unbreak.min.txt",
    ),
    ("easylist", "https://easylist.to/easylist/easylist.txt"),
    (
        "easyprivacy",
        "https://easylist.to/easylist/easyprivacy.txt",
    ),
    (
        "pgl",
        "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=adblockplus&showintro=0&mimetype=plaintext",
    ),
];

/// Subresources a typical page asks for: trackers, ads and first-party files.
const RESOURCES: [(&str, &str); 12] = [
    (
        "https://www.googletagmanager.com/gtm.js?id=GTM-ABCDE",
        "script",
    ),
    (
        "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
        "script",
    ),
    ("https://www.google-analytics.com/analytics.js", "script"),
    ("https://connect.facebook.net/en_US/fbevents.js", "script"),
    ("https://static.criteo.net/js/ld/publishertag.js", "script"),
    (
        "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
        "script",
    ),
    (
        "https://cdn.jsdelivr.net/npm/jquery@3/dist/jquery.min.js",
        "script",
    ),
    (
        "https://fonts.googleapis.com/css2?family=Inter",
        "stylesheet",
    ),
    (
        "https://upload.wikimedia.org/wikipedia/commons/a/a9/Example.jpg",
        "image",
    ),
    ("{site}static/app.js", "script"),
    ("{site}images/logo.png", "image"),
    ("{site}api/v1/feed", "xmlhttprequest"),
];

/// Rule counts tried, in order, to find WebKit's per-list limit.
const PROBE_SIZES: [usize; 7] = [
    50_000, 100_000, 150_000, 200_000, 300_000, 500_000, 1_000_000,
];
/// Shard size 10 §4.2 starts from.
const SPEC_SHARD: usize = 50_000;
const COMPILE_TIMEOUT: Duration = Duration::from_secs(600);
const CHECK_ROUNDS: usize = 20;

fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("mado-proto-lists")
}

/// The list text, downloaded with curl unless today's copy is cached.
fn fetch(id: &str, url: &str) -> Result<(String, bool), String> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{id}.txt"));
    let fresh = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < Duration::from_secs(24 * 3600));
    if !fresh {
        let status = std::process::Command::new("/usr/bin/curl")
            .args([
                "-sSfL",
                "--max-time",
                "30",
                "--max-filesize",
                "20971520",
                "-o",
            ])
            .arg(&file)
            .arg(url)
            .status()
            .map_err(|e| format!("curl: {e}"))?;
        if !status.success() {
            return Err(format!("{id}: download failed ({status})"));
        }
    }
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    Ok((text, fresh))
}

fn wait(rx: &mpsc::Receiver<Result<(), String>>, timeout: Duration) -> Result<(), String> {
    rx.recv_timeout(timeout)
        .map_err(|_| "WebKit never answered".to_owned())?
}

/// Compiles `json` as `id` and returns the seconds it took, or the error.
fn compile(ctx: &Ctx, id: &str, json: String) -> Result<f64, String> {
    let id = id.to_owned();
    let started = Instant::now();
    let rx = on_main(&ctx.app, move || macos::compile_rules(&id, &json))??;
    wait(&rx, COMPILE_TIMEOUT)?;
    Ok(started.elapsed().as_secs_f64())
}

fn lookup(ctx: &Ctx, id: &str) -> Result<f64, String> {
    let id = id.to_owned();
    let started = Instant::now();
    let rx = on_main(&ctx.app, move || macos::lookup_rules(&id))??;
    wait(&rx, STEP_TIMEOUT)?;
    Ok(started.elapsed().as_secs_f64())
}

fn json_array(rules: &[String]) -> String {
    let mut out =
        String::with_capacity(rules.iter().map(String::len).sum::<usize>() + rules.len() + 2);
    out.push('[');
    out.push_str(&rules.join(","));
    out.push(']');
    out
}

fn synthetic_rule(i: usize) -> String {
    format!(
        r#"{{"trigger":{{"url-filter":"^https?://ad{i}\\.example\\.com/"}},"action":{{"type":"block"}}}}"#
    )
}

/// This run's rule stores, removed when the run ends; compiled lists take
/// hundreds of megabytes across a run.
fn stores_root() -> PathBuf {
    std::env::temp_dir().join(format!("mado-proto-rules-{}", std::process::id()))
}

fn fresh_store(ctx: &Ctx, name: &str) -> Result<PathBuf, String> {
    let dir = stores_root().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let d = dir.clone();
    on_main(&ctx.app, move || macos::open_rule_store(&d))??;
    Ok(dir)
}

fn engine_numbers(set: &FilterSet) -> Result<Value, String> {
    let started = Instant::now();
    let engine = Engine::new_with_filter_set(set.clone());
    let build = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let bytes = engine.serialize();
    let serialize = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let mut loaded = Engine::default();
    loaded
        .deserialize(&bytes)
        .map_err(|e| format!("deserialize: {e:?}"))?;
    let deserialize = started.elapsed().as_secs_f64();

    let mut requests = Vec::new();
    for site in SITES {
        for (url, kind) in RESOURCES {
            let url = url.replace("{site}", site);
            if let Ok(r) = Request::new(&url, site, kind, "GET") {
                requests.push(r);
            }
        }
    }
    let mut times = Vec::with_capacity(requests.len() * CHECK_ROUNDS);
    let mut blocked = 0;
    for round in 0..CHECK_ROUNDS {
        for r in &requests {
            let t = Instant::now();
            let result = loaded.check_network_request(r);
            times.push(t.elapsed().as_secs_f64() * 1e6);
            if round == 0 && result.should_block() {
                blocked += 1;
            }
        }
    }
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let pct = |p: f64| times[((times.len() - 1) as f64 * p) as usize];
    Ok(json!({
        "buildSeconds": build,
        "serializedBytes": bytes.len(),
        "serializeSeconds": serialize,
        "deserializeSeconds": deserialize,
        "checks": times.len(),
        "requestsBlocked": format!("{blocked} of {}", requests.len()),
        "checkMicros": { "mean": mean, "p50": pct(0.5), "p99": pct(0.99), "max": pct(1.0) },
    }))
}

const ADS_PAGE: &str = r#"<!doctype html><title>Blocking fixture</title>
<img src="/ads/a1.png"><img src="/ads/a2.png"><img src="/ads/a3.png"><img src="/img/ok.png">"#;

fn fixture(path: &str) -> Option<String> {
    match path {
        "/page" => Some(ADS_PAGE.to_owned()),
        p if p.ends_with(".png") => Some(String::new()),
        _ => None,
    }
}

/// Experiment 7: does the private delegate method report blocked loads?
fn blocked_counts(ctx: &Ctx, checks: &mut Checks) -> Result<Value, String> {
    fresh_store(ctx, "fixture")?;
    let rule = r#"[{"trigger":{"url-filter":"/ads/"},"action":{"type":"block"}}]"#;
    compile(ctx, "fixture-ads", rule.to_owned())?;
    let server = Fixtures::start(fixture)?;
    let tab = ctx.tab;
    macos::wait_navigation(tab, STEP_TIMEOUT)?;
    on_main(&ctx.app, move || macos::apply_rules(tab, "fixture-ads"))??;
    if let Ok(mut l) = macos::RULE_ACTIONS.lock() {
        l.clear();
    }
    macos::clear_navigation(tab);
    let url = server.url("/page");
    on_main(&ctx.app, move || macos::navigate(tab, &url))??;
    macos::wait_navigation(tab, STEP_TIMEOUT)?;
    // Let late callbacks arrive.
    std::thread::sleep(Duration::from_millis(500));
    let actions = macos::RULE_ACTIONS
        .lock()
        .map(|l| l.clone())
        .unwrap_or_default();
    let paths: Vec<String> = server.requests().into_iter().map(|r| r.path).collect();
    let ads_reached = paths.iter().filter(|p| p.starts_with("/ads/")).count();
    checks.check(
        "the rule blocks: no /ads/ request reaches the server",
        ads_reached == 0 && paths.iter().any(|p| p == "/img/ok.png"),
        json!(paths),
    );
    checks.check(
        "the private delegate reports each blocked load (3)",
        actions
            .iter()
            .filter(|a| a.blocked_load == Some(true))
            .count()
            == 3,
        json!(actions),
    );
    Ok(json!({ "serverPaths": paths, "actions": actions }))
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let result = measure_all(ctx);
    let _ = std::fs::remove_dir_all(stores_root());
    result
}

fn measure_all(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();

    // Lists and engine.
    let mut set = FilterSet::new(true);
    let mut lists = Vec::new();
    for (id, url) in LISTS {
        let (text, cached) = fetch(id, url)?;
        lists.push(json!({ "id": id, "bytes": text.len(), "lines": text.lines().count(), "cached": cached }));
        set.add_filter_list(text, ParseOptions::default());
    }
    let engine = engine_numbers(&set)?;
    checks.check(
        "request check averages under 50 µs (10 §5.7)",
        engine["checkMicros"]["mean"]
            .as_f64()
            .is_some_and(|m| m < 50.0),
        engine["checkMicros"].clone(),
    );

    // Conversion to WebKit rules.
    let started = Instant::now();
    let (rules, converted) = set
        .into_content_blocking()
        .map_err(|()| "into_content_blocking failed")?;
    let convert_seconds = started.elapsed().as_secs_f64();
    let rules: Vec<String> = rules
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    // Per-list limit, with synthetic rules so the size is the only variable.
    let mut probes = Vec::new();
    let mut probe = |n: usize| -> Result<bool, String> {
        fresh_store(ctx, &format!("probe-{n}"))?;
        let synthetic: Vec<String> = (0..n).map(synthetic_rule).collect();
        let outcome = compile(ctx, &format!("probe-{n}"), json_array(&synthetic));
        probes.push(json!({ "rules": n, "seconds": outcome.as_ref().ok(), "error": outcome.as_ref().err() }));
        Ok(outcome.is_ok())
    };
    let mut limit = None;
    let mut failed_at = None;
    for &n in &PROBE_SIZES {
        if probe(n)? {
            limit = Some(n);
        } else {
            failed_at = Some(n);
            break;
        }
    }
    // Narrow the limit down to a single rule between the last size that
    // compiled and the first that did not.
    if let (Some(mut low), Some(mut high)) = (limit, failed_at) {
        while high - low > 1 {
            let mid = low + (high - low) / 2;
            if probe(mid)? {
                low = mid;
            } else {
                high = mid;
            }
        }
        limit = Some(low);
    }

    // Full default lists, cold, at the spec's shard size and at one shard.
    let mut full = Vec::new();
    for (name, shard) in [("spec", SPEC_SHARD), ("one", rules.len().max(1))] {
        fresh_store(ctx, &format!("full-{name}"))?;
        let mut cold = 0.0;
        let mut errors = Vec::new();
        let ids: Vec<String> = (0..rules.len().div_ceil(shard))
            .map(|i| format!("full-{name}-{i}"))
            .collect();
        for (id, chunk) in ids.iter().zip(rules.chunks(shard)) {
            match compile(ctx, id, json_array(chunk)) {
                Ok(s) => cold += s,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        let mut warm = 0.0;
        for id in &ids {
            warm += lookup(ctx, id).unwrap_or(f64::NAN);
        }
        full.push(json!({ "shardSize": shard, "shards": ids.len(), "coldSeconds": cold, "warmLookupSeconds": warm, "errors": errors }));
    }
    let spec_cold = full[0]["coldSeconds"].as_f64().unwrap_or(f64::INFINITY);
    checks.check(
        "full default lists compile cold in under 10 s (16 §5 row 6)",
        full[0]["errors"].as_array().is_some_and(Vec::is_empty) && spec_cold < 10.0,
        full[0].clone(),
    );

    let delegate = blocked_counts(ctx, &mut checks)?;

    Ok(checks.finish(json!({
        "lists": lists,
        "engine": engine,
        "contentRules": { "rules": rules.len(), "convertedFilters": converted.len(), "convertSeconds": convert_seconds },
        "limitProbes": probes,
        "largestCompiled": limit,
        "fullLists": full,
        "blockedCounts": delegate,
    })))
}
