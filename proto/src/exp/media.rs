//! The two macOS rows of 02's engine table still marked "(verify in M0)":
//! `SetMuted` (suspend with `setAllMediaPlaybackSuspended:`, mute with a page
//! script that sets `muted`) and `ShowDevTools` (a programmatic route to Web
//! Inspector). The audio is silent, so nobody hears the experiment.

use std::time::Duration;

use serde_json::{Value, json};

use super::Checks;
use crate::fixtures::Fixtures;
use crate::macos;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};

/// One second of silent 8 kHz, 8-bit mono WAV as a data URL.
fn silent_wav() -> String {
    const RATE: u32 = 8000;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + RATE).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes()); // byte rate
    wav.extend_from_slice(&1u16.to_le_bytes()); // block align
    wav.extend_from_slice(&8u16.to_le_bytes()); // bits
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend(std::iter::repeat_n(128u8, RATE as usize)); // 8-bit silence
    format!("data:audio/wav;base64,{}", base64(&wav))
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
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

fn page() -> String {
    format!(
        r#"<!doctype html><title>Media fixture</title><audio id="a" loop src="{}"></audio>"#,
        silent_wav()
    )
}

fn fixture(path: &str) -> Option<String> {
    (path == "/media").then(page)
}

const STATE: &str = "const a = document.querySelector('#a');
  return { paused: a.paused, muted: a.muted, volume: a.volume, time: a.currentTime };";

fn eval(ctx: &Ctx, tab: usize, body: &'static str) -> Result<Value, String> {
    let rx = on_main(&ctx.app, move || macos::eval_tab(tab, body))??;
    rx.recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "script never answered")?
}

fn webkit_state(ctx: &Ctx, tab: usize) -> Result<String, String> {
    let rx = on_main(&ctx.app, move || macos::media_state(tab))??;
    rx.recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "no media state".to_owned())
}

fn suspend(ctx: &Ctx, tab: usize, on: bool) -> Result<(), String> {
    let rx = on_main(&ctx.app, move || macos::suspend_media(tab, on))??;
    rx.recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "suspend never finished".to_owned())?;
    // Let the page see the change.
    std::thread::sleep(Duration::from_millis(300));
    Ok(())
}

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let server = Fixtures::start(fixture)?;
    macos::wait_navigation(ctx.tab, STEP_TIMEOUT)?;
    let window = ctx.window()?;
    let tab = on_main(&ctx.app, move || macos::add_media_tab(&window))??;
    macos::clear_navigation(tab);
    let url = server.url("/media");
    on_main(&ctx.app, move || macos::navigate(tab, &url))??;
    macos::wait_navigation(tab, STEP_TIMEOUT)?;

    let started = eval(
        ctx,
        tab,
        "await document.querySelector('#a').play(); await new Promise((r) => setTimeout(r, 300));",
    )
    .and_then(|_| eval(ctx, tab, STATE))?;
    let playing = webkit_state(ctx, tab)?;
    suspend(ctx, tab, true)?;
    let suspended = (eval(ctx, tab, STATE)?, webkit_state(ctx, tab)?);
    suspend(ctx, tab, false)?;
    let resumed = (eval(ctx, tab, STATE)?, webkit_state(ctx, tab)?);
    checks.check(
        "suspend: playback stops and WebKit reports it suspended, then resumes",
        started["paused"] == false
            && playing == "playing"
            && suspended.1 == "suspended"
            && resumed.1 == "playing",
        json!({ "started": started, "webkit": [playing, suspended.1, resumed.1], "page": [suspended.0, resumed.0] }),
    );

    let muted = eval(
        ctx,
        tab,
        "for (const m of document.querySelectorAll('audio, video')) m.muted = true;
         await new Promise((r) => setTimeout(r, 300));
         const a = document.querySelector('#a');
         return { paused: a.paused, muted: a.muted, volume: a.volume };",
    )?;
    let muted_state = webkit_state(ctx, tab)?;
    checks.check(
        "mute by page script: the element is muted and keeps playing",
        muted["muted"] == true && muted["paused"] == false && muted_state == "playing",
        json!({ "page": muted, "webkit": muted_state }),
    );

    let inspector = on_main(&ctx.app, move || macos::inspector_probe(tab))??;
    let send =
        |message: &'static str| on_main(&ctx.app, move || macos::inspector_send(tab, message))?;
    // Web Inspector loads in its own process, so visibility takes a moment.
    let visible_within = |want: bool| -> Result<Option<f64>, String> {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if send("isVisible")? == Some(want) {
                return Ok(Some(started.elapsed().as_secs_f64()));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(None)
    };
    let shown = if inspector["methods"]
        .as_array()
        .is_some_and(|m| m.iter().all(|e| e[1] == true))
    {
        // Docked first, so showing it stays inside the (transparent) tab
        // window instead of opening a window of its own.
        send("attach").and_then(|_| send("show")).and_then(|_| {
            let opened = visible_within(true)?;
            send("close")?;
            let closed = visible_within(false)?;
            Ok(json!({ "secondsToVisible": opened, "secondsToClosed": closed }))
        })
    } else {
        Err("`_inspector` lacks attach, show, close or isVisible".into())
    };
    let shown = shown.unwrap_or_else(|e| json!({ "error": e }));
    checks.check(
        "ShowDevTools: `_inspector` shows and closes Web Inspector programmatically",
        shown["secondsToVisible"].is_number() && shown["secondsToClosed"].is_number(),
        json!({ "probe": inspector, "showClose": shown }),
    );
    Ok(checks.finish(json!({ "inspector": inspector, "showClose": shown })))
}
