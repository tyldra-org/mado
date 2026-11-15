//! Updates (spec 15 §5): check the channel on launch and every 6 hours,
//! honour the staged rollout, download and verify in the background, then
//! install on "Relaunch" or, if the user never relaunches, when Mado quits,
//! so the next launch runs the new version.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mado_core::{CoreHandle, Input, Internal, Updates};
use mado_platform::Paths;
use mado_storage::Storage;
use mado_types::ProfileId;
use mado_types::command::OwnerAction;
use mado_types::command::{Command, Origin};
use mado_types::state::{UpdateState, UpdateStatus};
use rusqlite::OptionalExtension;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::mpsc;

const EVERY: Duration = Duration::from_secs(6 * 3600);
const FIRST_CHECK: Duration = Duration::from_secs(10);

/// A downloaded, verified update waiting to be installed.
pub type Staged = Arc<Mutex<Option<(Update, Vec<u8>)>>>;

#[derive(Debug)]
enum Ask {
    Check,
    /// `if_idle`: only when every other profile is idle (security).
    Relaunch {
        if_idle: bool,
    },
}

/// The core's handle on the updater task.
#[derive(Debug)]
pub struct TauriUpdates {
    tx: mpsc::UnboundedSender<Ask>,
}

impl Updates for TauriUpdates {
    fn check(&mut self) {
        let _ = self.tx.send(Ask::Check);
    }
    fn relaunch(&mut self) {
        let _ = self.tx.send(Ask::Relaunch { if_idle: false });
    }
    fn relaunch_when_idle(&mut self) {
        let _ = self.tx.send(Ask::Relaunch { if_idle: true });
    }
}

/// `hash(install_id) mod 1000 < rollout × 1000` (spec 15 §5). FNV-1a, so
/// the answer is the same on every launch and every machine.
pub fn in_rollout(install_id: &str, rollout: f64) -> bool {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in install_id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let bucket = hash % 1000;
    // rollout is 0..=1; 1000 buckets.
    #[expect(clippy::cast_precision_loss, reason = "bucket < 1000")]
    let bucket = bucket as f64;
    bucket < rollout.clamp(0.0, 1.0) * 1000.0
}

/// The random install ID in `app_state`, made on first use. Used for the
/// rollout and nothing else; never sent anywhere. Blocks on SQLite: call it
/// through `spawn_blocking`.
fn install_id(app_db: &Storage) -> Option<String> {
    let existing = app_db
        .readers()
        .read(|c| {
            c.query_row("SELECT v FROM app_state WHERE k = 'install_id'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()
        })
        .ok()
        .flatten();
    if existing.is_some() {
        return existing;
    }
    let id = uuid_v4();
    let value = id.clone();
    app_db
        .writer()
        .write(move |c| {
            c.execute(
                "INSERT OR IGNORE INTO app_state (k, v) VALUES ('install_id', ?1)",
                [value],
            )
        })
        .wait()
        .ok()?;
    Some(id)
}

fn uuid_v4() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn endpoint(channel: &str) -> String {
    std::env::var("MADO_UPDATE_URL").unwrap_or_else(|_| {
        format!("https://github.com/tyldra-org/mado/releases/download/channels/{channel}.json")
    })
}

fn restore_key(id: ProfileId) -> String {
    format!("restore_after_update.{id}")
}

/// Marks `id` to open its last session at its next launch (spec 15 §5).
pub fn mark_restore(app_db: &Storage, id: ProfileId) {
    if let Err(e) = crate::profiles::set_app_state(app_db, &restore_key(id), "1").wait() {
        tracing::warn!(error = %e, "the session restore mark was not written");
    }
}

/// Whether `id` quit for an update, clearing the mark.
pub fn take_restore(app_db: &Storage, id: ProfileId) -> bool {
    let key = restore_key(id);
    app_db
        .writer()
        .write(move |c| c.execute("DELETE FROM app_state WHERE k = ?1", [key]))
        .wait()
        .is_ok_and(|n| n > 0)
}

const RELAUNCH_KEY: &str = "relaunch_profiles";

/// The other profiles an update relaunch quit, to start again; cleared.
pub fn take_relaunch_list(app_db: &Storage) -> Vec<ProfileId> {
    let ids = app_db
        .readers()
        .read(|c| {
            c.query_row(
                "SELECT v FROM app_state WHERE k = ?1",
                [RELAUNCH_KEY],
                |r| r.get::<_, String>(0),
            )
            .optional()
        })
        .ok()
        .flatten();
    let _ = app_db
        .writer()
        .write(|c| c.execute("DELETE FROM app_state WHERE k = ?1", [RELAUNCH_KEY]))
        .wait();
    ids.unwrap_or_default()
        .split(',')
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// Quits every other running profile for the update (each marks its own
/// session to come back), remembers them, installs the staged update and
/// restarts this process, which starts them again. With `if_idle`, one
/// profile in use stops the relaunch. Returns false when it stopped.
/// Blocks: run it on a blocking thread.
pub fn relaunch_all(
    core: &CoreHandle,
    paths: &Paths,
    app_db: &Storage,
    own: ProfileId,
    staged: &Staged,
    if_idle: bool,
) -> bool {
    let others: Vec<ProfileId> = crate::profiles::list(app_db)
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.id)
        .filter(|id| *id != own && crate::lock::is_held(&paths.lock(*id)))
        .collect();
    let action = if if_idle {
        OwnerAction::QuitForUpdateIfIdle
    } else {
        OwnerAction::QuitForUpdate
    };
    let mut quit = Vec::new();
    for id in others {
        match crate::profile_service::tell(paths, id, action) {
            Ok(()) => quit.push(id),
            Err(e) if if_idle => {
                tracing::info!(%id, error = %e, "a profile is in use; the security update waits");
                // The ones already asked come back now.
                for back in quit {
                    let _ = crate::profile_service::launch(back, None);
                }
                return false;
            }
            Err(e) => tracing::warn!(%id, error = %e, "a profile did not quit for the update"),
        }
    }
    // Their locks go when they have written everything.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while quit.iter().any(|id| crate::lock::is_held(&paths.lock(*id)))
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    let list: Vec<String> = quit.iter().map(ToString::to_string).collect();
    let _ = crate::profiles::set_app_state(app_db, RELAUNCH_KEY, &list.join(",")).wait();
    mark_restore(app_db, own);
    let installed = install_staged(staged);
    if !installed {
        tracing::warn!("no update was installed; relaunching the same version");
    }
    // The new process waits for this one's lock and opens the session's
    // windows even if this one was started hidden; this one quits as on ⌘Q
    // without questions, so its session and the database are saved.
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    let mut old = std::env::args_os().skip(1);
    while let Some(a) = old.next() {
        if a == "--profile" {
            old.next();
        } else if a != "--relaunch" && a != "--hidden" {
            args.push(a);
        }
    }
    let spawned = std::env::current_exe().and_then(|exe| {
        std::process::Command::new(exe)
            .args(&args)
            .arg("--relaunch")
            .arg("--profile")
            .arg(own.to_string())
            .spawn()
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "Mado could not start again after the update");
        return false;
    }
    core.send(Input::Internal(Internal::Terminate));
    true
}

/// Starts the updater task. Returns the core's handle on it.
pub fn start(
    app: AppHandle,
    core: Arc<std::sync::OnceLock<CoreHandle>>,
    app_db: Arc<Storage>,
    staged: Staged,
    paths: Paths,
    own: ProfileId,
) -> TauriUpdates {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(async move {
        let mut next = tokio::time::Instant::now() + FIRST_CHECK;
        loop {
            let ask = tokio::select! {
                ask = rx.recv() => match ask { Some(a) => a, None => return },
                () = tokio::time::sleep_until(next) => Ask::Check,
            };
            let Some(core) = core.get() else { continue };
            match ask {
                Ask::Check => {
                    next = tokio::time::Instant::now() + EVERY;
                    if staged.lock().map(|s| s.is_some()).unwrap_or(false) {
                        continue;
                    }
                    let status = check(&app, core, &app_db, &staged).await;
                    core.send(Input::Internal(Internal::UpdateStatus(status)));
                }
                Ask::Relaunch { if_idle } => {
                    let has_update = staged.lock().map(|s| s.is_some()).unwrap_or(false);
                    if has_update {
                        let (core, paths, db, staged) = (
                            core.clone(),
                            paths.clone(),
                            Arc::clone(&app_db),
                            Arc::clone(&staged),
                        );
                        let _ = tauri::async_runtime::spawn_blocking(move || {
                            relaunch_all(&core, &paths, &db, own, &staged, if_idle)
                        })
                        .await;
                    }
                }
            }
        }
    });
    TauriUpdates { tx }
}

fn status(status: UpdateStatus, version: Option<String>, security: bool) -> UpdateState {
    UpdateState {
        status,
        version,
        security,
    }
}

async fn check(
    app: &AppHandle,
    core: &CoreHandle,
    app_db: &Arc<Storage>,
    staged: &Staged,
) -> UpdateState {
    let channel = core
        .command(
            Origin::System,
            Command::SettingsGet {
                keys: Some(vec!["update.channel".into()]),
            },
        )
        .await
        .ok()
        .and_then(Result::ok)
        .and_then(|v| v["update.channel"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "stable".into());
    core.send(Input::Internal(Internal::UpdateStatus(status(
        UpdateStatus::Checking,
        None,
        false,
    ))));
    let found = async {
        let url = endpoint(&channel)
            .parse()
            .map_err(|e: url::ParseError| e.to_string())?;
        let updater = app
            .updater_builder()
            .endpoints(vec![url])
            .map_err(|e| e.to_string())?
            .build()
            .map_err(|e| e.to_string())?;
        updater.check().await.map_err(|e| e.to_string())
    }
    .await;
    let update = match found {
        Ok(Some(update)) => update,
        Ok(None) => return status(UpdateStatus::Idle, None, false),
        Err(e) => {
            tracing::warn!(error = %e, %channel, "the update check failed");
            return status(UpdateStatus::Error, None, false);
        }
    };
    let rollout = update.raw_json["rollout"].as_f64().unwrap_or(1.0);
    let security = update.raw_json["security"].as_bool().unwrap_or(false);
    let version = Some(update.version.clone());
    let db = Arc::clone(app_db);
    let id = tauri::async_runtime::spawn_blocking(move || install_id(&db))
        .await
        .ok()
        .flatten();
    if !id.is_some_and(|id| in_rollout(&id, rollout)) {
        tracing::info!(version = %update.version, rollout, "not in this update's rollout yet");
        return status(UpdateStatus::Idle, None, false);
    }
    core.send(Input::Internal(Internal::UpdateStatus(status(
        UpdateStatus::Downloading,
        version.clone(),
        security,
    ))));
    // The plugin verifies the minisign signature before returning the bytes.
    match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => {
            if let Ok(mut slot) = staged.lock() {
                *slot = Some((update, bytes));
            }
            status(UpdateStatus::Ready, version, security)
        }
        Err(e) => {
            tracing::warn!(error = %e, "the update download failed");
            status(UpdateStatus::Error, version, security)
        }
    }
}

/// Replaces the app bundle with the staged update. True when it did.
pub fn install_staged(staged: &Staged) -> bool {
    let Some((update, bytes)) = staged.lock().ok().and_then(|mut s| s.take()) else {
        return false;
    };
    match update.install(&bytes) {
        Ok(()) => {
            tracing::info!(version = %update.version, "update installed");
            true
        }
        Err(e) => {
            tracing::error!(error = %e, "the update did not install");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollout_is_stable_and_proportional() {
        let ids: Vec<String> = (0..10_000).map(|i| format!("install-{i}")).collect();
        let share = |r: f64| ids.iter().filter(|id| in_rollout(id, r)).count();
        assert_eq!(share(0.0), 0);
        assert_eq!(share(1.0), 10_000);
        let tenth = share(0.1);
        assert!((800..1200).contains(&tenth), "{tenth}");
        assert_eq!(in_rollout("same", 0.5), in_rollout("same", 0.5));
        // Raising the rollout never drops someone already in it.
        assert!(
            ids.iter()
                .filter(|id| in_rollout(id, 0.1))
                .all(|id| in_rollout(id, 0.5))
        );
    }

    /// A channel file and two archives on a local server, as the release
    /// workflow publishes them.
    fn serve(files: Vec<(&'static str, Vec<u8>)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader, Write};
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let path = line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).map_or(true, |n| n == 0) || h == "\r\n" {
                        break;
                    }
                }
                let body = files
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map(|(_, b)| b.clone());
                let mut s = &stream;
                let _ = match body {
                    Some(b) => s
                        .write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len()).as_bytes())
                        .and_then(|()| s.write_all(&b)),
                    None => s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                };
            }
        });
        format!("http://{addr}")
    }

    /// 15 §10 check 2: the plugin, configured as Mado's (signed version
    /// required), downloads a correctly signed archive and refuses a
    /// tampered one.
    #[test]
    fn a_tampered_archive_is_rejected() {
        use base64::Engine as _;
        use tauri_plugin_updater::UpdaterExt as _;
        let b64 = |s: String| base64::engine::general_purpose::STANDARD.encode(s);
        let keys = minisign::KeyPair::generate_unencrypted_keypair().expect("keys");
        let archive = b"the update archive".to_vec();
        let signature = minisign::sign(
            Some(&keys.pk),
            &keys.sk,
            std::io::Cursor::new(&archive),
            Some("timestamp:0\tfile:Mado.app.tar.gz\tversion:9.0.0"),
            None,
        )
        .expect("sign")
        .into_string();
        let mut tampered = archive.clone();
        tampered[0] ^= 1;
        let channel = |url: &str| {
            serde_json::json!({
                "version": "9.0.0",
                "pub_date": "2026-11-15T00:00:00Z",
                "rollout": 1,
                "security": false,
                "platforms": { "darwin-aarch64": { "url": url, "signature": b64(signature.clone()) } }
            })
        };
        let base = serve(vec![
            ("/good.tar.gz", archive.clone()),
            ("/bad.tar.gz", tampered),
        ]);
        let good = channel(&format!("{base}/good.tar.gz"))
            .to_string()
            .into_bytes();
        let bad = channel(&format!("{base}/bad.tar.gz"))
            .to_string()
            .into_bytes();
        let base2 = serve(vec![("/good.json", good), ("/bad.json", bad)]);
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".into(),
            serde_json::json!({
                "pubkey": b64(keys.pk.to_box().expect("box").to_string()),
                "endpoints": [],
                "requireSignedVersion": true,
                "dangerousInsecureTransportProtocol": true
            }),
        );
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .expect("app");
        let fetch = |name: &str| {
            let url: url::Url = format!("{base2}/{name}").parse().expect("url");
            tauri::async_runtime::block_on(async {
                let update = app
                    .updater_builder()
                    .endpoints(vec![url])
                    .expect("endpoints")
                    .build()
                    .expect("updater")
                    .check()
                    .await
                    .expect("check")
                    .expect("an update");
                update.download(|_, _| {}, || {}).await
            })
        };
        assert_eq!(fetch("good.json").expect("verified"), archive);
        assert!(
            fetch("bad.json").is_err(),
            "the tampered archive must be refused"
        );
    }

    /// 15 §10 check 3: with `rollout: 0`, no install ID is in, so no
    /// client downloads or installs it.
    #[test]
    fn rollout_zero_reaches_nobody() {
        assert!((0..10_000).all(|i| !in_rollout(&format!("id-{i}"), 0.0)));
    }

    #[test]
    fn install_ids_are_v4_uuids() {
        let id = uuid_v4();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
    }
}
