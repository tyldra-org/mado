//! Installing and removing extensions (spec 09 §2): from the Chrome Web
//! Store page's "Add to Mado", and in Developer mode from a folder, a
//! `.crx` or a `.zip`. Downloads, verification and unpacking run off the
//! core thread; the install prompt decides.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mado_engine::page::StoreInstall;
use mado_engine::{EngineOp, TabCommand, World};
use mado_extensions::store::{MAX_PACKAGE, download};
use mado_extensions::{Expect, Manifest, Refusal};
use mado_types::command::{Command, ExtensionSource, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::state::{ExtensionInstallPrompt, InstalledExtension, Notice, PromptState};
use mado_types::{ErrorCode, ExtensionId, MadoError, PromptId, TabId, WindowId};
use rusqlite::params;
use serde_json::{Value, json};

use super::{Core, Internal};

/// The store's host: its pages get the `store` script's labels.
const STORE_HOST: &str = "chromewebstore.google.com";
/// More pinned extensions than this go to the extensions menu (09 §2.2).
const MAX_PINNED: usize = 6;
/// The largest icon shown in the prompt.
const MAX_ICON: u64 = 256 * 1024;

/// Where packages come from and where installed ones go.
#[derive(Clone)]
pub struct ExtensionsConfig {
    /// `<profile>/Extensions/`.
    pub dir: PathBuf,
    /// A store extension's package URL.
    pub package_url: Arc<dyn Fn(&str) -> String + Send + Sync>,
    /// Publisher keys accepted besides the store's: tests only.
    pub extra_publishers: Vec<[u8; 32]>,
}

impl std::fmt::Debug for ExtensionsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionsConfig")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

impl ExtensionsConfig {
    /// The Chrome Web Store, extensions in `dir`.
    pub fn store(dir: PathBuf) -> Self {
        Self {
            dir,
            package_url: Arc::new(mado_extensions::store::package_url),
            extra_publishers: Vec::new(),
        }
    }
}

/// A package ready for the prompt.
#[derive(Debug)]
pub struct Staged {
    id: ExtensionId,
    source: ExtensionSource,
    /// The unpacked files: a staging folder, or the folder itself for an
    /// unpacked extension.
    files: PathBuf,
    manifest: Manifest,
    icon: Option<String>,
}

/// Why a package was not offered: the notice's message key.
type Refused = &'static str;

/// A `data:` URL of the package's largest icon.
fn icon_of(files: &Path, manifest: &Manifest) -> Option<String> {
    use base64::Engine as _;
    let rel = manifest.icon()?.trim_start_matches('/');
    if rel.split('/').any(|p| p == "..") {
        return None;
    }
    let path = files.join(rel);
    if std::fs::metadata(&path).ok()?.len() > MAX_ICON {
        return None;
    }
    let mime = if rel.ends_with(".png") {
        "image/png"
    } else if rel.ends_with(".svg") {
        return None;
    } else {
        "image/jpeg"
    };
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn refusal_key(r: &Refusal) -> Refused {
    match r {
        Refusal::Theme => "notice.extensionTheme",
        Refusal::App | Refusal::TooOld(_) => "notice.extensionUnsupported",
        Refusal::Malformed(_) => "notice.extensionMalformed",
    }
}

/// A fresh folder under `<dir>/.staging/`.
fn staging(dir: &Path) -> PathBuf {
    dir.join(".staging").join(uuid::Uuid::now_v7().to_string())
}

/// Reads the manifest of unpacked files.
fn read(files: PathBuf, id: ExtensionId, source: ExtensionSource) -> Result<Staged, Refused> {
    let manifest = Manifest::read(&files, "en").map_err(|e| refusal_key(&e))?;
    let icon = icon_of(&files, &manifest);
    Ok(Staged {
        id,
        source,
        files,
        manifest,
        icon,
    })
}

/// Verifies and unpacks a CRX (09 §2.2 steps 3-5).
fn stage_crx(
    bytes: &[u8],
    expect: Expect<'_>,
    dir: &Path,
    source: ExtensionSource,
) -> Result<Staged, Refused> {
    let verified = mado_extensions::verify(bytes, expect).map_err(|e| {
        tracing::warn!(error = %e, "a package did not verify");
        "notice.extensionUnverified"
    })?;
    let id: ExtensionId = verified
        .id
        .parse()
        .map_err(|_| "notice.extensionUnverified")?;
    let files = staging(dir);
    if let Some(parent) = files.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "notice.extensionFailed")?;
    }
    if let Err(e) = mado_extensions::unzip(verified.zip, &files) {
        tracing::warn!(error = %e, "a package did not unpack");
        let _ = std::fs::remove_dir_all(&files);
        return Err("notice.extensionMalformed");
    }
    read(files.clone(), id, source).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&files);
    })
}

/// An unpacked folder's ID: from the manifest's `key`, else its path.
fn unpacked_id(folder: &Path) -> Option<ExtensionId> {
    use base64::Engine as _;
    let text = std::fs::read_to_string(folder.join("manifest.json")).ok()?;
    let key = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}'))
        .ok()?
        .get("key")
        .and_then(Value::as_str)
        .and_then(|k| base64::engine::general_purpose::STANDARD.decode(k).ok());
    let id = match key {
        Some(spki) => mado_extensions::id_from_key(&spki),
        None => mado_extensions::id_from_path(folder),
    };
    id.parse().ok()
}

/// A Developer mode install from `path` (09 §2.3).
fn stage_local(path: &Path, source: ExtensionSource, dir: &Path) -> Result<Staged, Refused> {
    let path = path
        .canonicalize()
        .map_err(|_| "notice.extensionNotFound")?;
    match source {
        ExtensionSource::Unpacked => {
            let id = unpacked_id(&path).ok_or("notice.extensionMalformed")?;
            read(path, id, source)
        }
        ExtensionSource::CrxFile => {
            let meta = std::fs::metadata(&path).map_err(|_| "notice.extensionNotFound")?;
            if meta.len() > MAX_PACKAGE as u64 {
                return Err("notice.extensionMalformed");
            }
            let bytes = std::fs::read(&path).map_err(|_| "notice.extensionNotFound")?;
            stage_crx(
                &bytes,
                Expect {
                    id: None,
                    from_store: false,
                    extra_publishers: &[],
                },
                dir,
                source,
            )
        }
        ExtensionSource::Zip => {
            let bytes = std::fs::read(&path).map_err(|_| "notice.extensionNotFound")?;
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.replace(|c: char| !c.is_alphanumeric() && c != '-', "_"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "extension".into());
            let unpacked = dir.join("Unpacked");
            std::fs::create_dir_all(&unpacked).map_err(|_| "notice.extensionFailed")?;
            let mut folder = unpacked.join(&stem);
            let mut n = 1;
            while folder.exists() {
                n += 1;
                folder = unpacked.join(format!("{stem}-{n}"));
            }
            mado_extensions::unzip(&bytes, &folder).map_err(|_| "notice.extensionMalformed")?;
            let id = unpacked_id(&folder).ok_or("notice.extensionMalformed")?;
            read(folder, id, ExtensionSource::Zip)
        }
        ExtensionSource::Store => Err("notice.extensionFailed"),
    }
}

/// The `extensions` rows (spec 03).
pub(super) fn load(
    conn: &rusqlite::Connection,
) -> rusqlite::Result<Vec<(InstalledExtension, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, version, enabled, source, path, pinned FROM extensions ORDER BY installed_at, id",
    )?;
    let rows = stmt.query_map([], |r| {
        let source: String = r.get(4)?;
        let path: Option<String> = r.get(5)?;
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, bool>(3)?,
            match (source.as_str(), path) {
                ("store", _) => ExtensionSource::Store,
                ("crx_file", _) => ExtensionSource::CrxFile,
                (_, Some(p)) if p.contains("/Extensions/Unpacked/") => ExtensionSource::Zip,
                _ => ExtensionSource::Unpacked,
            },
            r.get::<_, bool>(6)?,
        ))
    })?;
    Ok(rows
        .filter_map(Result::ok)
        .filter_map(|(id, name, version, enabled, source, pinned)| {
            Some((
                InstalledExtension {
                    id: id.parse().ok()?,
                    name,
                    version,
                    enabled,
                    source,
                },
                pinned,
            ))
        })
        .collect())
}

impl Core {
    fn extensions_dir(&self) -> Option<PathBuf> {
        self.extensions.as_ref().map(|c| c.dir.clone())
    }

    /// `extension.install` and `extension.confirmInstall`.
    pub(super) fn extension_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        if !origin.is_user() {
            return Err(MadoError::new(
                ErrorCode::Protected,
                "error.extension.notAllowed",
            ));
        }
        if self.state.profile.is_private || self.extensions.is_none() {
            return Err(MadoError::new(
                ErrorCode::InvalidInput,
                "error.extension.private",
            ));
        }
        let window = match origin {
            Origin::Ui { window } | Origin::Overlay { window } => Some(*window),
            _ => self.current_window(),
        };
        match cmd {
            Command::ExtensionInstall {
                id,
                source: ExtensionSource::Store,
                ..
            } => {
                let id = id.ok_or_else(|| {
                    MadoError::new(ErrorCode::InvalidInput, "error.extension.noId")
                })?;
                self.store_install(id, window, rec);
                Ok(Value::Null)
            }
            Command::ExtensionInstall { source, path, .. } => {
                if self.setting("extensions.developerMode") != true {
                    return Err(MadoError::new(
                        ErrorCode::InvalidInput,
                        "error.extension.developerMode",
                    ));
                }
                let path = path.ok_or_else(|| {
                    MadoError::new(ErrorCode::InvalidInput, "error.extension.noPath")
                })?;
                let dir = self.extensions_dir().unwrap_or_default();
                let inputs = self.inputs.clone();
                tokio::task::spawn_local(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        stage_local(Path::new(&path), source, &dir)
                    })
                    .await
                    .unwrap_or(Err("notice.extensionFailed"));
                    let _ = inputs.send(super::Input::Internal(Internal::ExtensionStaged {
                        result: result.map(Box::new),
                        window,
                    }));
                });
                Ok(Value::Null)
            }
            Command::ExtensionConfirmInstall { prompt_id, add } => {
                let Some(staged) = self.extension_installs.remove(&prompt_id) else {
                    return Err(MadoError::new(ErrorCode::NotFound, "error.prompt.notFound"));
                };
                let mut prompts = self.state.prompts.clone();
                prompts.remove(&prompt_id);
                self.state.tx(rec).set_prompts(prompts);
                if add {
                    self.add_extension(staged, rec);
                } else if staged.source != ExtensionSource::Unpacked {
                    let _ = std::fs::remove_dir_all(&staged.files);
                }
                Ok(Value::Null)
            }
            _ => Err(MadoError::new(
                ErrorCode::InvalidInput,
                "error.command.unknown",
            )),
        }
    }

    fn extension_notice(&mut self, key: &str, name: Option<&str>, rec: &mut PatchRecorder) {
        let id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let notice = Notice {
            id,
            message_key: key.into(),
            args: name
                .map(|n| BTreeMap::from([("name".to_owned(), json!(n))]))
                .unwrap_or_default(),
            action: None,
            undo: None,
            expires_at: Some(self.now() + 8000),
        };
        self.add_notice(notice, rec);
    }

    /// 09 §2.2 steps 1-5 for a store extension.
    fn store_install(
        &mut self,
        id: ExtensionId,
        window: Option<WindowId>,
        rec: &mut PatchRecorder,
    ) {
        let compat = mado_lists::compat();
        if compat.blocked.contains_key(id.as_str()) {
            self.extension_notice("notice.extensionBlocked", None, rec);
            return;
        }
        if compat
            .extensions
            .get(id.as_str())
            .is_some_and(|e| e.category == mado_lists::Category::Theme)
        {
            self.extension_notice("notice.extensionTheme", None, rec);
            return;
        }
        let Some(config) = self.extensions.clone() else {
            return;
        };
        let inputs = self.inputs.clone();
        tokio::task::spawn_local(async move {
            let url = (config.package_url)(id.as_str());
            let result = match mado_extensions::store::client() {
                Ok(client) => download(&client, &url, MAX_PACKAGE).await.map_err(|e| {
                    tracing::warn!(error = %e, "an extension download failed");
                    "notice.extensionDownloadFailed"
                }),
                Err(_) => Err("notice.extensionDownloadFailed"),
            };
            let result = match result {
                Ok(bytes) => tokio::task::spawn_blocking(move || {
                    stage_crx(
                        &bytes,
                        Expect {
                            id: Some(id.as_str()),
                            from_store: true,
                            extra_publishers: &config.extra_publishers,
                        },
                        &config.dir,
                        ExtensionSource::Store,
                    )
                })
                .await
                .unwrap_or(Err("notice.extensionFailed")),
                Err(e) => Err(e),
            };
            let _ = inputs.send(super::Input::Internal(Internal::ExtensionStaged {
                result: result.map(Box::new),
                window,
            }));
        });
    }

    /// A package is ready: show the install prompt (09 §2.2 steps 6-7).
    pub(super) fn extension_staged(
        &mut self,
        result: Result<Box<Staged>, Refused>,
        window: Option<WindowId>,
        rec: &mut PatchRecorder,
    ) {
        let staged = match result {
            Ok(s) => *s,
            Err(key) => {
                self.extension_notice(key, None, rec);
                return;
            }
        };
        let compat = mado_lists::compat();
        if compat.blocked.contains_key(staged.id.as_str()) {
            if staged.source != ExtensionSource::Unpacked {
                let _ = std::fs::remove_dir_all(&staged.files);
            }
            self.extension_notice("notice.extensionBlocked", None, rec);
            return;
        }
        let summary = staged
            .manifest
            .summary(mado_extensions::manifest::SUPPORTED_PERMISSIONS);
        let vouched = compat.extensions.get(staged.id.as_str()).is_some_and(|e| {
            matches!(
                e.status,
                mado_lists::Status::Tested | mado_lists::Status::Mostly
            )
        });
        let replaces = self
            .state
            .extensions
            .installed
            .iter()
            .find(|e| e.id == staged.id)
            .map(|e| e.version.clone());
        let id = PromptId::new();
        let prompt = ExtensionInstallPrompt {
            id,
            window_id: window,
            extension_id: staged.id.clone(),
            name: staged.manifest.name.clone(),
            version: staged.manifest.version.clone(),
            icon: staged.icon.clone(),
            permissions: summary.permissions,
            hosts: summary.hosts,
            all_sites: summary.all_sites,
            optional: summary.optional,
            unavailable: summary.unavailable,
            may_not_work: summary.missing_required && !vouched,
            older_format: staged.manifest.manifest_version < 3,
            replaces,
        };
        let mut prompts = self.state.prompts.clone();
        prompts.insert(id, PromptState::ExtensionInstall(prompt));
        self.state.tx(rec).set_prompts(prompts);
        self.extension_installs.insert(id, staged);
    }

    /// Add: the files move into the profile and the row is written (09
    /// §2.2 steps 8 and 11). Loading in the engine arrives with M3-T04.
    fn add_extension(&mut self, staged: Staged, rec: &mut PatchRecorder) {
        let Some(dir) = self.extensions_dir() else {
            return;
        };
        let id = staged.id.clone();
        let m = &staged.manifest;
        let path = match staged.source {
            ExtensionSource::Unpacked | ExtensionSource::Zip => Some(staged.files.clone()),
            ExtensionSource::Store | ExtensionSource::CrxFile => {
                let home = dir.join(id.as_str());
                let dest = home.join(&m.version);
                let moved = (|| {
                    std::fs::create_dir_all(&home)?;
                    if dest.exists() {
                        std::fs::remove_dir_all(&dest)?;
                    }
                    std::fs::rename(&staged.files, &dest)?;
                    for old in std::fs::read_dir(&home)?.flatten() {
                        if old.path() != dest {
                            let _ = std::fs::remove_dir_all(old.path());
                        }
                    }
                    Ok::<_, std::io::Error>(())
                })();
                if let Err(e) = moved {
                    tracing::error!(error = %e, "an extension could not be installed");
                    let _ = std::fs::remove_dir_all(&staged.files);
                    self.extension_notice("notice.extensionFailed", Some(&m.name), rec);
                    return;
                }
                None
            }
        };
        let summary = m.summary(mado_extensions::manifest::SUPPORTED_PERMISSIONS);
        let mut ext = self.state.extensions.clone();
        let pin = m.has_action
            && !ext.pinned.contains(&id)
            && ext.pinned.len() < MAX_PINNED
            && !ext.installed.iter().any(|e| e.id == id);
        let entry = InstalledExtension {
            id: id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            enabled: true,
            source: staged.source,
        };
        match ext.installed.iter_mut().find(|e| e.id == id) {
            Some(e) => *e = entry,
            None => ext.installed.push(entry),
        }
        if pin {
            ext.pinned.push(id.clone());
        }
        self.state.tx(rec).set_extensions(ext);
        let now = self.now();
        let row = (
            id.to_string(),
            m.version.clone(),
            m.name.clone(),
            staged.source.column(),
            if summary.all_sites { "all" } else { "specific" },
            json!(summary.permissions).to_string(),
            json!(summary.hosts).to_string(),
            pin,
            m.update_url.clone(),
            path.map(|p| p.to_string_lossy().into_owned()),
        );
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "INSERT INTO extensions (id, version, name, source, enabled, site_access, granted_permissions, granted_hosts, pinned, installed_at, updated_at, update_url, path)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?9, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET version = ?2, name = ?3, source = ?4, site_access = ?5,
                   granted_permissions = ?6, granted_hosts = ?7, updated_at = ?9, update_url = ?10, path = ?11",
                params![row.0, row.1, row.2, row.3, row.4, row.5, row.6, row.7, now, row.8, row.9],
            )
            .map(|_| ())
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "an extension row was not saved");
            }
        });
        self.extension_notice("notice.extensionAdded", Some(&staged.manifest.name), rec);
        self.refresh_store_pages();
    }

    /// "Remove from Mado" on the store page: the row and Mado's copy of
    /// the files go.
    fn remove_extension(&mut self, id: &ExtensionId, rec: &mut PatchRecorder) {
        let mut ext = self.state.extensions.clone();
        let Some(pos) = ext.installed.iter().position(|e| &e.id == id) else {
            return;
        };
        let removed = ext.installed.remove(pos);
        ext.pinned.retain(|p| p != id);
        ext.toolbar.retain(|b| &b.id != id);
        self.state.tx(rec).set_extensions(ext);
        if matches!(
            removed.source,
            ExtensionSource::Store | ExtensionSource::CrxFile
        ) && let Some(dir) = self.extensions_dir()
        {
            let _ = std::fs::remove_dir_all(dir.join(id.as_str()));
        }
        let key = id.to_string();
        let pending = self.profile_db.writer().write(move |c| {
            c.execute("DELETE FROM extensions WHERE id = ?1", [&key])?;
            c.execute(
                "DELETE FROM extension_shortcuts WHERE extension_id = ?1",
                [&key],
            )
            .map(|_| ())
        });
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "an extension row was not removed");
            }
        });
        self.extension_notice("notice.extensionRemoved", Some(&removed.name), rec);
        self.refresh_store_pages();
    }

    fn on_store_page(&self, tab: TabId) -> bool {
        self.model
            .tabs
            .get(&tab)
            .and_then(|t| url::Url::parse(&t.url).ok())
            .is_some_and(|u| u.scheme() == "https" && u.host_str() == Some(STORE_HOST))
    }

    /// The `store` script's message (09 §2.1): only the ID is used, only
    /// from a store page in the main frame.
    pub(super) fn store_page_message(
        &mut self,
        tab: TabId,
        msg: StoreInstall,
        rec: &mut PatchRecorder,
    ) {
        if !self.on_store_page(tab) || self.state.profile.is_private {
            return;
        }
        let Ok(id) = msg.id.parse::<ExtensionId>() else {
            return;
        };
        if msg.remove {
            self.remove_extension(&id, rec);
        } else {
            let window = self.model.tabs.get(&tab).map(|t| t.window_id);
            self.store_install(id, window, rec);
        }
    }

    /// Tells the `store` script on `tab` what is installed and unavailable.
    pub(super) fn store_page_loaded(&mut self, tab: TabId) {
        if !self.on_store_page(tab) {
            return;
        }
        let compat = mado_lists::compat();
        let unavailable: Vec<&String> = compat
            .blocked
            .keys()
            .chain(
                compat
                    .extensions
                    .iter()
                    .filter(|(_, e)| e.category == mado_lists::Category::Theme)
                    .map(|(id, _)| id),
            )
            .collect();
        let installed: Vec<&str> = self
            .state
            .extensions
            .installed
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        let s = |k: &str| self.settings.strings().get(k).to_owned();
        let known = json!({
            "installed": installed,
            "unavailable": unavailable,
            "labels": {
                "add": s("store.add"),
                "remove": s("store.remove"),
                "unavailable": s("store.unavailable"),
            },
        });
        let source = format!(
            "globalThis[Symbol.for(\"mado.store\")] = {known}; globalThis[Symbol.for(\"mado.storeUpdate\")]?.(); return null;"
        );
        self.outbox.push(EngineOp::Apply(
            tab,
            TabCommand::EvaluateInWorld {
                world: World::Mado,
                source,
                reply: Box::new(|_| {}),
            },
        ));
    }

    fn refresh_store_pages(&mut self) {
        let tabs: Vec<TabId> = self.model.tabs.keys().copied().collect();
        for tab in tabs {
            self.store_page_loaded(tab);
        }
    }
}
