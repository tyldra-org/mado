//! The daily fetch of the signed lists (spec 09 §7, 15 §7). Every profile
//! process schedules it; the shared `<root>/Lists/fetched-at` stamp lets
//! only the first one in a day go to the network.

use std::sync::Arc;
use std::time::Duration;

use mado_lists::{Installed, List, Store};
use mado_types::patch::PatchRecorder;

use super::{Core, Internal, Timer};

/// How often the lists are fetched.
const EVERY: Duration = Duration::from_secs(24 * 3600);
/// Not before the app is up.
const FIRST: Duration = Duration::from_secs(60);
/// After a failed fetch.
const RETRY: Duration = Duration::from_secs(3600);
/// Signatures are four short lines.
const MAX_SIGNATURE: usize = 4 * 1024;
/// The largest list, the HSTS preload list, is under 1 MB.
const MAX_LIST: usize = 16 * 1024 * 1024;

/// Where the lists come from and where the last good copies go.
#[derive(Debug, Clone)]
pub struct ListsConfig {
    pub store: Arc<Store>,
    /// The published folder, ending in `/` ([`mado_lists::URL`]).
    pub url: String,
}

/// One fetch's outcome.
#[derive(Debug, Default)]
pub struct Fetched {
    pub updated: Vec<List>,
    /// A list could not be downloaded; the fetch is tried again sooner.
    pub unreachable: bool,
}

async fn get(client: &reqwest::Client, url: &str, max: usize) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > max {
            return Err(format!("{url}: over {max} bytes"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Downloads one list when its signature claims a newer copy, and
/// installs it if it verifies.
async fn fetch_one(
    client: &reqwest::Client,
    config: &ListsConfig,
    list: List,
) -> Result<Option<Installed>, String> {
    let file = list.file();
    let signature = get(
        client,
        &format!("{}{file}.minisig", config.url),
        MAX_SIGNATURE,
    )
    .await?;
    let signature =
        String::from_utf8(signature).map_err(|_| format!("{file}.minisig: not text"))?;
    if mado_lists::signed_serial(&signature).is_some_and(|s| s <= mado_lists::serial(list)) {
        return Ok(None);
    }
    let bytes = get(client, &format!("{}{file}", config.url), MAX_LIST).await?;
    let store = Arc::clone(&config.store);
    let installed = tokio::task::spawn_blocking(move || store.install(list, &bytes, &signature))
        .await
        .map_err(|e| e.to_string())?;
    match installed {
        Ok(i) => Ok(Some(i)),
        // A list that fails its checks is reported, and the last good copy
        // stays; trying again sooner would not change the answer.
        Err(e) => {
            tracing::warn!(error = %e, "a fetched list was rejected");
            Ok(None)
        }
    }
}

async fn fetch_all(config: ListsConfig) -> Fetched {
    let mut fetched = Fetched::default();
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "no HTTP client for the lists");
            fetched.unreachable = true;
            return fetched;
        }
    };
    for list in List::ALL {
        match fetch_one(&client, &config, list).await {
            Ok(Some(Installed::Updated(serial))) => {
                tracing::info!(list = list.file(), serial, "list updated");
                fetched.updated.push(list);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(list = list.file(), error = %e, "list fetch failed");
                fetched.unreachable = true;
            }
        }
    }
    fetched
}

impl Core {
    /// Makes the last good copies current, off the core thread, then
    /// schedules the fetch.
    pub(super) fn load_lists(&mut self) {
        let Some(config) = &self.lists else { return };
        let store = Arc::clone(&config.store);
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            let loaded = tokio::task::spawn_blocking(move || store.load())
                .await
                .unwrap_or_default();
            let mut updated = Vec::new();
            for (list, result) in loaded {
                match result {
                    Ok(Installed::Updated(serial)) => {
                        tracing::info!(list = list.file(), serial, "list loaded");
                        updated.push(list);
                    }
                    Ok(Installed::Unchanged) => {}
                    Err(e) => tracing::warn!(error = %e, "a saved list was not used"),
                }
            }
            let _ = inputs.send(super::Input::Internal(Internal::ListsLoaded(updated)));
        });
        self.schedule_lists_fetch();
    }

    pub(super) fn lists_loaded(&mut self, updated: &[List], rec: &mut PatchRecorder) {
        if updated.contains(&List::Compat) {
            self.compat_changed(rec);
        }
    }

    /// Schedules the next fetch from the shared stamp.
    pub(super) fn schedule_lists_fetch(&mut self) {
        let Some(config) = &self.lists else { return };
        let wait = config.store.next_fetch_in(self.now(), EVERY).max(FIRST);
        self.timers.schedule(Timer::ListsFetch, wait);
    }

    pub(super) fn fetch_lists(&mut self) {
        let Some(config) = self.lists.clone() else {
            return;
        };
        if !config.store.next_fetch_in(self.now(), EVERY).is_zero() {
            // Another profile fetched meanwhile.
            self.schedule_lists_fetch();
            return;
        }
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            let fetched = fetch_all(config).await;
            let _ = inputs.send(super::Input::Internal(Internal::ListsFetched(fetched)));
        });
    }

    pub(super) fn lists_fetched(&mut self, fetched: Fetched, rec: &mut PatchRecorder) {
        let now = self.now();
        let Some(config) = &self.lists else { return };
        if fetched.unreachable {
            self.timers.schedule(Timer::ListsFetch, RETRY);
        } else {
            if let Err(e) = config.store.fetched(now) {
                tracing::warn!(error = %e, "the lists fetch stamp was not written");
            }
            self.timers.schedule(Timer::ListsFetch, EVERY);
        }
        self.lists_loaded(&fetched.updated, rec);
    }

    /// The compatibility list changed: About's engine check follows it.
    pub(super) fn compat_changed(&mut self, rec: &mut PatchRecorder) {
        let mut about = self.state.about.clone();
        about.engine_outdated = crate::about::engine_outdated(&about.engine_version);
        if about != self.state.about {
            self.state.tx(rec).set_about(about);
        }
    }
}
