//! Experiment 14 (16 §5 row 14, M0-T05): memory of one profile process with
//! 1, 10 and 50 tabs, and whether a tab's WebContent process can be found for
//! per-tab memory (04 §4).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::Checks;
use super::sites::SITES;
use crate::shell::{Ctx, on_main};
use crate::{macos, procs};

/// How long a site may take to load before it counts as failed.
const LOAD_TIMEOUT: Duration = Duration::from_secs(45);
/// Quiet time after loading, before measuring, so memory settles.
const SETTLE: Duration = Duration::from_secs(10);
const STEPS: [usize; 3] = [1, 10, 50];

fn measure(ctx: &Ctx, tabs: &[usize]) -> Result<Value, String> {
    let ids = tabs.to_vec();
    let pids = on_main(&ctx.app, move || {
        ids.iter()
            .map(|&t| macos::web_process_id(t).map(|p| (t, p)))
            .collect::<Result<Vec<_>, String>>()
    })??;
    let family = procs::family();
    let total: u64 = family.iter().filter_map(|p| p.footprint).sum();
    let mut by_kind: BTreeMap<String, (u64, usize)> = BTreeMap::new();
    for p in &family {
        let kind = if p.pid == std::process::id() as i32 {
            "app".to_owned()
        } else {
            p.name.clone()
        };
        let e = by_kind.entry(kind).or_default();
        e.0 += p.footprint.unwrap_or(0);
        e.1 += 1;
    }
    let distinct: std::collections::BTreeSet<i32> = pids.iter().filter_map(|(_, p)| *p).collect();
    Ok(json!({
        "tabs": tabs.len(),
        "totalFootprintMiB": total as f64 / 1048576.0,
        "byKind": by_kind.iter().map(|(k, (b, n))| json!({ "kind": k, "processes": n, "footprintMiB": *b as f64 / 1048576.0 })).collect::<Vec<_>>(),
        "webContentPids": pids,
        "distinctWebContentProcesses": distinct.len(),
    }))
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let window = ctx.window()?;
    let started = Instant::now();
    let mut tabs: Vec<usize> = Vec::new();
    let mut loads = Vec::new();
    let mut steps = Vec::new();
    for &step in &STEPS {
        // Open the next tabs together, as a session restore would.
        let mut opened = Vec::new();
        for site in &SITES[tabs.len()..step] {
            let w = window.clone();
            let first = tabs.is_empty() && opened.is_empty();
            let tab = if first {
                ctx.tab
            } else {
                on_main(&ctx.app, move || macos::add_tab(&w, true))??
            };
            macos::clear_navigation(tab);
            let url = (*site).to_owned();
            on_main(&ctx.app, move || macos::navigate(tab, &url))??;
            opened.push((tab, *site, Instant::now()));
        }
        for (tab, site, at) in opened {
            let result = macos::wait_navigation(tab, LOAD_TIMEOUT);
            loads.push(json!({
                "site": site,
                "ok": result.is_ok(),
                "seconds": at.elapsed().as_secs_f64(),
                "error": result.err(),
            }));
            tabs.push(tab);
        }
        std::thread::sleep(SETTLE);
        steps.push(measure(ctx, &tabs)?);
    }
    let failed = loads.iter().filter(|l| l["ok"] == false).count();
    checks.check(
        "at least 45 of 50 sites loaded",
        failed <= 5,
        json!({ "failed": failed }),
    );
    let found = steps
        .last()
        .and_then(|s| s["webContentPids"].as_array())
        .is_some_and(|p| p.iter().all(|e| e[1].is_i64()));
    checks.check(
        "every tab's WebContent PID is readable (04 §4)",
        found,
        json!(
            steps
                .last()
                .map(|s| s["distinctWebContentProcesses"].clone())
        ),
    );
    Ok(checks.finish(json!({
        "steps": steps,
        "loads": loads,
        "minutes": started.elapsed().as_secs_f64() / 60.0,
    })))
}
