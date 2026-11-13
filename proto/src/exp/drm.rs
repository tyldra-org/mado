//! Experiment 9 (16 §5 row 9, M0-T06), the part that needs no subscription:
//! which DRM key systems WKWebView offers to pages in the self-signed build.
//! Playing each streaming service needs its account, so that part waits for
//! the maintainer.

use serde_json::{Value, json};

use super::Checks;
use crate::macos;
use crate::shell::{Ctx, STEP_TIMEOUT, on_main};

/// Asks EME for each key system with an H.264 video and AAC audio
/// configuration, and the older WebKit-prefixed API for FairPlay.
const PROBE: &str = "
  const config = [{ initDataTypes: ['sinf', 'skd', 'cenc'],
    videoCapabilities: [{ contentType: 'video/mp4; codecs=\"avc1.42E01E\"' }],
    audioCapabilities: [{ contentType: 'audio/mp4; codecs=\"mp4a.40.2\"' }] }];
  const systems = ['com.apple.fps', 'com.apple.fps.1_0', 'com.apple.fps.2_0',
                   'com.widevine.alpha', 'com.microsoft.playready', 'org.w3.clearkey'];
  const eme = {};
  for (const s of systems) {
    try { await navigator.requestMediaKeySystemAccess(s, config); eme[s] = 'supported'; }
    catch (e) { eme[s] = e.name; }
  }
  const prefixed = typeof WebKitMediaKeys !== 'undefined'
    ? WebKitMediaKeys.isTypeSupported('com.apple.fps.1_0', 'video/mp4') : null;
  return { eme, webkitMediaKeysFairPlay: prefixed, userAgent: navigator.userAgent,
           secure: isSecureContext };";

pub fn run(ctx: &Ctx) -> Result<Value, String> {
    let mut checks = Checks::new();
    let tab = ctx.tab;
    macos::wait_navigation(tab, STEP_TIMEOUT)?;
    // EME needs a secure context; the shell starts on https://example.com/.
    let rx = on_main(&ctx.app, move || macos::eval_tab(tab, PROBE))??;
    let probe = rx
        .recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "script never answered")??;
    checks.check(
        "FairPlay (com.apple.fps) is offered to pages",
        probe["eme"]["com.apple.fps"] == "supported",
        probe.clone(),
    );
    Ok(checks.finish(json!({ "keySystems": probe })))
}
