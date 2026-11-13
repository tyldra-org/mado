//! Experiment 11 (11 §2): does a Keychain item written by one self-signed
//! build stay readable, with no prompt, by the next build signed with the
//! same identity? And does a build with a different signature get refused?
//! User interaction is disabled for every Keychain call, so where macOS
//! would prompt the call fails with errSecInteractionNotAllowed (-25308)
//! instead: nothing ever appears on screen. The item lives in a throwaway
//! keychain file, deleted at the end; access control works the same way in
//! the login keychain.
//!
//! Needs the other builds next to this one: `proto/build-app.sh MadoProto2 2`
//! (same identity, new version and binary) and
//! `proto/build-app.sh MadoProtoAdhoc 3 adhoc` (ad hoc signature).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
use serde_json::{Value, json};

use super::Checks;
use crate::shell::Ctx;

const PATH_ENV: &str = "MADO_PROTO_KEYCHAIN";
const SECRET_ENV: &str = "MADO_PROTO_KEYCHAIN_SECRET";
/// The throwaway keychain's password; it holds only the test item.
const PASSWORD: &str = "mado-proto-m0";
const SERVICE: &str = "app.mado.vault";
const ACCOUNT: &str = "m0-experiment-11";
/// errSecInteractionNotAllowed: macOS would have shown a prompt.
const WOULD_PROMPT: i64 = -25308;
/// errSecAuthFailed: the item's access control refused the caller.
const AUTH_FAILED: i64 = -25293;

fn status(e: &security_framework::base::Error) -> Value {
    json!({ "code": e.code(), "message": e.to_string() })
}

/// What `codesign` says about a bundle: designated requirement and CDHash.
fn signature(app: &Path) -> Value {
    let run = |args: &[&str]| {
        std::process::Command::new("/usr/bin/codesign")
            .args(args)
            .arg(app)
            .output()
            .map(|o| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
            })
            .unwrap_or_default()
    };
    let details = run(&["-dvvv"]);
    let field = |name: &str| {
        details
            .lines()
            .find_map(|l| l.strip_prefix(name))
            .map(str::to_owned)
    };
    json!({
        "requirement": run(&["-d", "-r-"]).lines().find(|l| l.starts_with("designated")).map(str::to_owned),
        "cdhash": field("CDHash="),
        "signature": field("Signature="),
        "authority": field("Authority="),
    })
}

pub fn run(_ctx: &Ctx) -> Result<Value, String> {
    if let (Ok(path), Ok(secret)) = (std::env::var(PATH_ENV), std::env::var(SECRET_ENV)) {
        return read_here(&path, &secret);
    }
    let mut checks = Checks::new();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let this_app = exe
        .ancestors()
        .nth(3)
        .ok_or("not inside an app bundle")?
        .to_path_buf();
    let target = this_app.parent().ok_or("no target folder")?;
    let update = target.join("MadoProto2.app");
    let adhoc = target.join("MadoProtoAdhoc.app");
    for app in [&update, &adhoc] {
        if !app.exists() {
            return Err(format!(
                "{} is missing: run proto/build-app.sh MadoProto2 2 and proto/build-app.sh MadoProtoAdhoc 3 adhoc",
                app.display()
            ));
        }
    }
    let dir = std::env::temp_dir().join(format!("mado-proto-keychain-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("m0.keychain-db");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let secret = format!("m0-{nanos}");

    let written = {
        let _no_prompts = SecKeychain::disable_user_interaction().map_err(|e| e.to_string())?;
        let keychain = CreateOptions::new()
            .password(PASSWORD)
            .create(&path)
            .map_err(|e| e.to_string())?;
        keychain
            .set_generic_password(SERVICE, ACCOUNT, secret.as_bytes())
            .map_err(|e| e.to_string())?;
        match keychain.find_generic_password(SERVICE, ACCOUNT) {
            Ok((password, _)) => json!({ "readBack": password.as_ref() == secret.as_bytes() }),
            Err(e) => json!({ "readBack": false, "error": status(&e) }),
        }
    };
    checks.check(
        "build 1 writes the item and reads it back",
        written["readBack"] == true,
        json!({ "write": written, "signature": signature(&this_app) }),
    );

    let read_by = |app: &PathBuf| -> Value {
        let out = dir.join(format!(
            "{}.json",
            app.file_stem()
                .map(|s| s.to_string_lossy())
                .unwrap_or_default()
        ));
        let status = std::process::Command::new(app.join("Contents/MacOS/mado-proto"))
            .args(["--exp", "keychain", "--out"])
            .arg(&out)
            .env(PATH_ENV, &path)
            .env(SECRET_ENV, &secret)
            .status();
        let child: Value = std::fs::read(&out)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        json!({
            "exit": status.ok().and_then(|s| s.code()),
            "read": child["result"]["observations"],
            "build": child["environment"]["build"],
            "signature": signature(app),
        })
    };
    let by_update = read_by(&update);
    checks.check(
        "build 2, same self-signed identity, reads the item with no prompt",
        by_update["read"]["read"] == true && by_update["read"]["matches"] == true,
        by_update.clone(),
    );
    let by_adhoc = read_by(&adhoc);
    checks.check(
        "a build with another signature cannot read the item without the user, so access is tied to the signing identity",
        by_adhoc["read"]["read"] == false
            && [WOULD_PROMPT, AUTH_FAILED].contains(&by_adhoc["read"]["error"]["code"].as_i64().unwrap_or(0)),
        by_adhoc.clone(),
    );
    let deleted = std::process::Command::new("/usr/bin/security")
        .arg("delete-keychain")
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success());
    let _ = std::fs::remove_dir_all(&dir);
    Ok(checks.finish(json!({ "keychainDeleted": deleted })))
}

/// The other builds' half: open the keychain and read the item, never
/// prompting.
fn read_here(path: &str, secret: &str) -> Result<Value, String> {
    let mut checks = Checks::new();
    let _no_prompts = SecKeychain::disable_user_interaction().map_err(|e| e.to_string())?;
    let mut keychain = SecKeychain::open(path).map_err(|e| e.to_string())?;
    keychain.unlock(Some(PASSWORD)).map_err(|e| e.to_string())?;
    let read = match keychain.find_generic_password(SERVICE, ACCOUNT) {
        Ok((password, _)) => {
            json!({ "read": true, "matches": password.as_ref() == secret.as_bytes() })
        }
        Err(e) => json!({ "read": false, "error": status(&e) }),
    };
    checks.check("the keychain opens and unlocks", true, Value::Null);
    Ok(checks.finish(read))
}
