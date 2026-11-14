//! Updates (spec 15 §5): check the channel on launch and every 6 hours,
//! honour the staged rollout, download and verify in the background, then
//! install on "Relaunch" or, if the user never relaunches, when Mado quits,
//! so the next launch runs the new version.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mado_core::{CoreHandle, Input, Internal, Updates};
use mado_storage::Storage;
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
    Relaunch,
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
        let _ = self.tx.send(Ask::Relaunch);
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
/// rollout and nothing else; never sent anywhere.
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

/// Starts the updater task. Returns the core's handle on it.
pub fn start(
    app: AppHandle,
    core: Arc<std::sync::OnceLock<CoreHandle>>,
    app_db: Arc<Storage>,
    staged: Staged,
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
                Ask::Relaunch => {
                    if install_staged(&staged) {
                        app.restart();
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
    app_db: &Storage,
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
    if !install_id(app_db).is_some_and(|id| in_rollout(&id, rollout)) {
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

    #[test]
    fn install_ids_are_v4_uuids() {
        let id = uuid_v4();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
    }
}
