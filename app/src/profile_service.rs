//! The app's side of profile commands (spec 12 §1): rows in the app
//! database, folders, other profiles' processes, and deletion with the
//! website data store and Keychain items.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mado_core::Profiles;
use mado_platform::Paths;
use mado_storage::Storage;
use mado_types::command::{LaunchRequest, LaunchSource, LinkRule, OwnerAction};
use mado_types::state::ProfileInfo;
use mado_types::{ErrorCode, MadoError, ProfileId};
use rusqlite::params;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::lock;
use crate::profiles::{self, Profile};

/// How long a profile's process gets to quit before deletion gives up.
const QUIT_WAIT: Duration = Duration::from_secs(20);

fn failed(e: impl std::fmt::Display) -> MadoError {
    MadoError::new(ErrorCode::Internal, "error.profile.failed").arg("reason", e.to_string())
}

fn not_found() -> MadoError {
    MadoError::new(ErrorCode::NotFound, "error.profile.notFound")
}

/// Removes the WebKit data store and the Keychain items of a profile; the
/// app passes the real ones, tests fakes.
pub struct Removers {
    pub data_store: Box<dyn Fn(ProfileId) -> Result<(), String> + Send + Sync>,
    pub keychain: Box<dyn Fn(ProfileId) -> Result<(), String> + Send + Sync>,
}

/// Starts a profile's process, with an optional URL.
pub type Launch = Box<dyn Fn(ProfileId, Option<&str>) -> std::io::Result<()> + Send + Sync>;

pub struct AppProfiles {
    pub app_db: Arc<Storage>,
    pub paths: Paths,
    pub own: ProfileId,
    pub removers: Removers,
    /// Starts a profile's process: `mado --profile <id> [url]`.
    pub launch: Launch,
    pub now: fn() -> i64,
}

impl std::fmt::Debug for AppProfiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppProfiles")
            .field("own", &self.own)
            .finish_non_exhaustive()
    }
}

/// The real removers: WebKit on the main thread, the profile's Keychain
/// service.
pub fn removers(app: AppHandle) -> Removers {
    Removers {
        data_store: Box::new(move |id| {
            let (tx, rx) = std::sync::mpsc::channel();
            app.run_on_main_thread(move || {
                mado_engine_webkit::remove_data_store(
                    id,
                    Box::new(move |r| {
                        let _ = tx.send(r.map_err(|e| e.to_string()));
                    }),
                );
            })
            .map_err(|e| e.to_string())?;
            rx.recv_timeout(Duration::from_secs(30))
                .map_err(|e| e.to_string())?
        }),
        keychain: Box::new(|id| {
            mado_platform::keychain::delete_service(&mado_platform::keychain::profile_service(id))
                .map(|_| ())
                .map_err(|e| e.to_string())
        }),
    }
}

/// `mado --profile <id> [url]`, which starts the profile or forwards to it.
pub fn launch(id: ProfileId, url: Option<&str>) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--profile").arg(id.to_string());
    if let Some(url) = url {
        cmd.arg(url);
    }
    cmd.spawn().map(|_| ())
}

impl AppProfiles {
    fn find(&self, id: ProfileId) -> Result<Profile, MadoError> {
        profiles::list(&self.app_db)
            .map_err(failed)?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(not_found)
    }

    fn running(&self, id: ProfileId) -> bool {
        lock::is_held(&self.paths.lock(id))
    }

    /// Asks a running profile's owner to act, through its control endpoint.
    fn tell(&self, id: ProfileId, action: OwnerAction) -> Result<(), String> {
        tell(&self.paths, id, action)
    }
}

/// Asks the owner of a running profile to act, through its control
/// endpoint (spec 12 §1.2).
pub fn tell(paths: &Paths, id: ProfileId, action: OwnerAction) -> Result<(), String> {
    tauri::async_runtime::block_on(tell_async(paths, id, action))
}

/// [`tell`], from async code.
pub async fn tell_async(paths: &Paths, id: ProfileId, action: OwnerAction) -> Result<(), String> {
    {
        let secret = mado_control::LaunchSecret::read(&paths.launch_secret(id))
            .map_err(|e| e.to_string())?;
        let request = LaunchRequest {
            urls: Vec::new(),
            files: Vec::new(),
            new_window: false,
            private: false,
            source: LaunchSource::Cli,
            control: Some(action),
        };
        let socket = paths.control_socket(id);
        mado_control::forward(&socket, &secret, &request)
            .await
            .map_err(|e| e.to_string())
    }
}

impl Profiles for AppProfiles {
    fn list(&self) -> Result<Value, MadoError> {
        let rows = profiles::list(&self.app_db).map_err(failed)?;
        Ok(json!(
            rows.iter()
                .map(|p| json!({
                    "id": p.id,
                    "name": p.name,
                    "color": p.color,
                    "avatar": p.avatar,
                    "current": p.id == self.own,
                    "running": p.id == self.own || self.running(p.id),
                }))
                .collect::<Vec<_>>()
        ))
    }

    fn create(&self, name: &str, color: &str, avatar: &str) -> Result<Value, MadoError> {
        let id = ProfileId::new();
        let (text, name, color, avatar, now) = (
            id.to_string(),
            name.to_owned(),
            color.to_owned(),
            avatar.to_owned(),
            (self.now)(),
        );
        self.app_db
            .writer()
            .write(move |c| {
                c.execute(
                    "INSERT INTO profiles (id, name, color, avatar, sort, created_at, schema_version)
                     VALUES (?1, ?2, ?3, ?4, (SELECT COALESCE(MAX(sort), 0) + 1 FROM profiles), ?5, 1)",
                    params![text, name, color, avatar, now],
                )
            })
            .wait()
            .map_err(failed)?;
        std::fs::create_dir_all(self.paths.profile(id)).map_err(failed)?;
        (self.launch)(id, None).map_err(failed)?;
        Ok(json!({ "id": id }))
    }

    fn update(
        &self,
        id: ProfileId,
        name: Option<String>,
        color: Option<String>,
        avatar: Option<String>,
    ) -> Result<ProfileInfo, MadoError> {
        self.find(id)?;
        let text = id.to_string();
        self.app_db
            .writer()
            .write(move |c| {
                c.execute(
                    "UPDATE profiles SET name = COALESCE(?2, name), color = COALESCE(?3, color), avatar = COALESCE(?4, avatar) WHERE id = ?1",
                    params![text, name, color, avatar],
                )
            })
            .wait()
            .map_err(failed)?;
        if id != self.own
            && self.running(id)
            && let Err(e) = self.tell(id, OwnerAction::ProfileChanged)
        {
            tracing::warn!(error = %e, "the other profile did not hear of its change");
        }
        Ok(self.find(id)?.info())
    }

    /// Spec 12 §1.4 steps 2 to 5.
    fn delete(&self, id: ProfileId) -> Result<(), MadoError> {
        if id == self.own {
            return Err(MadoError::new(
                ErrorCode::InvalidInput,
                "error.profile.current",
            ));
        }
        self.find(id)?;
        if self.running(id) {
            self.tell(id, OwnerAction::Quit).map_err(failed)?;
            let started = Instant::now();
            while self.running(id) {
                if started.elapsed() > QUIT_WAIT {
                    return Err(MadoError::new(
                        ErrorCode::Internal,
                        "error.profile.stillRunning",
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        (self.removers.data_store)(id).map_err(failed)?;
        (self.removers.keychain)(id).map_err(failed)?;
        let folder = self.paths.profile(id);
        if folder.exists() {
            std::fs::remove_dir_all(&folder).map_err(failed)?;
        }
        let _ = std::fs::remove_file(self.paths.control_entry(id));
        let text = id.to_string();
        self.app_db
            .writer()
            .write(move |c| {
                c.execute("DELETE FROM link_rules WHERE profile_id = ?1", [&text])?;
                c.execute("DELETE FROM profiles WHERE id = ?1", [&text])
            })
            .wait()
            .map_err(failed)?;
        // Never no profile: the asking one still exists, but keep the rule.
        profiles::ensure_one(&self.app_db, (self.now)()).map_err(failed)?;
        Ok(())
    }

    fn open(&self, id: ProfileId) -> Result<(), MadoError> {
        self.find(id)?;
        (self.launch)(id, None).map_err(failed)
    }

    fn link_rules(&self) -> Result<Vec<LinkRule>, MadoError> {
        let rows: Vec<(String, String)> = self
            .app_db
            .readers()
            .read(|c| {
                let mut stmt =
                    c.prepare("SELECT pattern, profile_id FROM link_rules ORDER BY sort")?;
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect()
            })
            .map_err(failed)?;
        Ok(rows
            .into_iter()
            .filter_map(|(pattern, id)| {
                Some(LinkRule {
                    pattern,
                    profile_id: id.parse().ok()?,
                })
            })
            .collect())
    }

    fn set_link_rules(&self, rules: Vec<LinkRule>) -> Result<(), MadoError> {
        let known: Vec<ProfileId> = profiles::list(&self.app_db)
            .map_err(failed)?
            .into_iter()
            .map(|p| p.id)
            .collect();
        if rules.iter().any(|r| !known.contains(&r.profile_id)) {
            return Err(not_found());
        }
        self.app_db
            .writer()
            .write(move |c| {
                c.execute("DELETE FROM link_rules", [])?;
                for (i, r) in rules.iter().enumerate() {
                    c.execute(
                        "INSERT INTO link_rules (id, pattern, profile_id, sort) VALUES (?1, ?2, ?3, ?4)",
                        params![uuid::Uuid::now_v7().to_string(), r.pattern, r.profile_id.to_string(), i64::try_from(i).unwrap_or(0)],
                    )?;
                }
                Ok(())
            })
            .wait()
            .map_err(failed)
    }

    fn route(&self, url: &str) -> Option<ProfileId> {
        profiles::route(&self.app_db, url)
    }

    fn export(&self, id: ProfileId, path: &str) -> Result<(), MadoError> {
        let profile = self.find(id)?;
        export(
            &self.paths,
            &profile,
            std::path::Path::new(path),
            (self.now)(),
        )
        .map_err(failed)
    }

    fn import(&self, path: &str) -> Result<Value, MadoError> {
        let id = ProfileId::new();
        let (name, color) = import(&self.paths, id, std::path::Path::new(path)).map_err(|e| {
            let _ = std::fs::remove_dir_all(self.paths.profile(id));
            MadoError::new(ErrorCode::InvalidInput, "error.profile.badFile").arg("reason", e)
        })?;
        let (text, now) = (id.to_string(), (self.now)());
        self.app_db
            .writer()
            .write(move |c| {
                c.execute(
                    "INSERT INTO profiles (id, name, color, avatar, sort, created_at, schema_version)
                     VALUES (?1, ?2, ?3, '', (SELECT COALESCE(MAX(sort), 0) + 1 FROM profiles), ?4, 1)",
                    params![text, name, color, now],
                )
            })
            .wait()
            .map_err(failed)?;
        Ok(json!({ "id": id }))
    }
}

/// Version of the `.madoprofile` format (spec 12 §1.5).
const EXPORT_VERSION: u32 = 1;

/// Tables an export leaves empty: page text, the session journal and the
/// AI audit stay on this Mac.
const NOT_EXPORTED: &[&str] = &["page_text", "page_text_meta", "session_journal", "ai_audit"];

/// Writes `profile`'s `.madoprofile` to `dest` (spec 12 §1.5): the
/// manifest, a copy of the database without page text, the session journal
/// and the AI audit, and the extension list. The vault joins with M4.
pub fn export(
    paths: &Paths,
    profile: &Profile,
    dest: &std::path::Path,
    now: i64,
) -> Result<(), String> {
    let copy = std::env::temp_dir().join(format!("mado-export-{}-{now}.sqlite", profile.id));
    let _ = std::fs::remove_file(&copy);
    let result = write_export(paths, profile, &copy, dest, now);
    let _ = std::fs::remove_file(&copy);
    result
}

fn write_export(
    paths: &Paths,
    profile: &Profile,
    copy: &std::path::Path,
    dest: &std::path::Path,
    now: i64,
) -> Result<(), String> {
    use std::io::Write;
    {
        let conn = rusqlite::Connection::open_with_flags(
            paths.profile_db(profile.id),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?;
        conn.execute("VACUUM INTO ?1", [copy.to_string_lossy()])
            .map_err(|e| e.to_string())?;
    }
    let extensions: Vec<Value> = {
        let c = rusqlite::Connection::open(copy).map_err(|e| e.to_string())?;
        for t in NOT_EXPORTED {
            c.execute(&format!("DELETE FROM {t}"), [])
                .map_err(|e| e.to_string())?;
        }
        let list = {
            let mut stmt = c
                .prepare("SELECT id, enabled, private_allowed FROM extensions ORDER BY id")
                .map_err(|e| e.to_string())?;
            stmt.query_map([], |r| {
                Ok(json!({ "id": r.get::<_, String>(0)?, "enabled": r.get::<_, bool>(1)?, "privateAllowed": r.get::<_, bool>(2)? }))
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect()
        };
        c.execute("DELETE FROM extensions", [])
            .map_err(|e| e.to_string())?;
        c.execute_batch("VACUUM").map_err(|e| e.to_string())?;
        list
    };
    let manifest = json!({ "version": EXPORT_VERSION, "name": profile.name, "color": profile.color, "created": now });
    let partial = dest.with_extension("madoprofile.partial");
    let file = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())
    };
    put("manifest.json", manifest.to_string().as_bytes())?;
    put(
        "profile.sqlite",
        &std::fs::read(copy).map_err(|e| e.to_string())?,
    )?;
    put("extensions.json", json!(extensions).to_string().as_bytes())?;
    zip.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&partial, dest).map_err(|e| e.to_string())
}

/// Reads a `.madoprofile` into a new profile folder; returns its name and
/// colour. The database's own version check refuses a newer one when the
/// profile opens.
pub fn import(
    paths: &Paths,
    id: ProfileId,
    src: &std::path::Path,
) -> Result<(String, String), String> {
    use std::io::Read;
    const MAX: u64 = 4 * 1024 * 1024 * 1024;
    let file = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut read = |name: &str| -> Result<Vec<u8>, String> {
        let entry = zip
            .by_name(name)
            .map_err(|_| format!("{name} is missing"))?;
        if entry.size() > MAX {
            return Err(format!("{name} is too large"));
        }
        let mut out = Vec::new();
        entry
            .take(MAX)
            .read_to_end(&mut out)
            .map_err(|e| e.to_string())?;
        Ok(out)
    };
    let manifest: Value =
        serde_json::from_slice(&read("manifest.json")?).map_err(|e| e.to_string())?;
    if manifest["version"].as_u64() != Some(u64::from(EXPORT_VERSION)) {
        return Err("not a profile file this Mado reads".into());
    }
    let db = read("profile.sqlite")?;
    if !db.starts_with(b"SQLite format 3\0") {
        return Err("the profile database is damaged".into());
    }
    let folder = paths.profile(id);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    std::fs::write(paths.profile_db(id), db).map_err(|e| e.to_string())?;
    let name: String = manifest["name"]
        .as_str()
        .unwrap_or("Imported")
        .chars()
        .take(64)
        .collect();
    let color = manifest["color"]
        .as_str()
        .filter(|c| c.len() <= 16)
        .unwrap_or("blue")
        .to_owned();
    Ok((name, color))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn service(
        dir: &std::path::Path,
        own: ProfileId,
        removed: Arc<Mutex<Vec<String>>>,
    ) -> AppProfiles {
        let paths = Paths::at(dir);
        let db = mado_storage::open(
            &paths.app_db(),
            mado_storage::Kind::App,
            &dir.join("b"),
            &dir.join("x"),
        )
        .expect("app db");
        let app_db = Arc::new(Storage::start(&db).expect("storage"));
        let (a, b) = (Arc::clone(&removed), removed);
        AppProfiles {
            app_db,
            paths,
            own,
            removers: Removers {
                data_store: Box::new(move |id| {
                    a.lock().expect("lock").push(format!("store {id}"));
                    Ok(())
                }),
                keychain: Box::new(move |id| {
                    b.lock().expect("lock").push(format!("keychain {id}"));
                    Ok(())
                }),
            },
            launch: Box::new(|_, _| Ok(())),
            now: || 1,
        }
    }

    /// 12 §7 check 2: deleting a profile removes its folder, its website data
    /// store and its Keychain items, then its rows and rules.
    #[test]
    fn deleting_a_profile_removes_everything_it_owns() {
        let dir = tempfile::tempdir().expect("dir");
        let removed = Arc::new(Mutex::new(Vec::new()));
        let mut s = service(dir.path(), ProfileId::new(), Arc::clone(&removed));
        let home = profiles::ensure_one(&s.app_db, 1).expect("one")[0].id;
        s.own = home;
        let work: ProfileId =
            serde_json::from_value(s.create("Work", "red", "").expect("create")["id"].clone())
                .expect("an id");
        std::fs::write(s.paths.profile(work).join("profile.sqlite"), b"x").expect("a file");
        s.set_link_rules(vec![LinkRule {
            pattern: "*.acme.com".into(),
            profile_id: work,
        }])
        .expect("rules");
        assert_eq!(s.route("https://docs.acme.com/x"), Some(work));
        assert_eq!(
            s.delete(home).expect_err("not itself").message_key,
            "error.profile.current"
        );
        s.delete(work).expect("delete");
        assert!(!s.paths.profile(work).exists());
        assert_eq!(
            *removed.lock().expect("lock"),
            vec![format!("store {work}"), format!("keychain {work}")]
        );
        assert!(s.link_rules().expect("rules").is_empty());
        let left: Vec<ProfileId> = profiles::list(&s.app_db)
            .expect("list")
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(left, vec![home]);
    }

    /// 12 §1.5: an export holds the bookmarks but no page text or session
    /// journal, and imports as a new profile.
    #[test]
    fn a_profile_exports_and_imports_without_private_tables() {
        let dir = tempfile::tempdir().expect("dir");
        let s = service(
            dir.path(),
            ProfileId::new(),
            Arc::new(Mutex::new(Vec::new())),
        );
        let home = profiles::ensure_one(&s.app_db, 1).expect("one")[0].clone();
        let db = mado_storage::open(
            &s.paths.profile_db(home.id),
            mado_storage::Kind::Profile,
            &s.paths.backups(home.id),
            &s.paths.broken(home.id),
        )
        .expect("profile db");
        drop(db);
        {
            let c = rusqlite::Connection::open(s.paths.profile_db(home.id)).expect("open");
            c.execute(
                "INSERT INTO bookmarks (id, parent_id, kind, title, url, sort, root, created_at, updated_at) VALUES ('b1', NULL, 'link', 'Kept', 'https://a.example/', 0, NULL, 1, 1)",
                [],
            )
            .expect("a bookmark");
            c.execute("INSERT INTO page_text (rowid, title, url, body) VALUES (1, 't', 'u', 'secret words')", [])
                .expect("page text");
        }
        let file = dir.path().join("Home.madoprofile");
        export(&s.paths, &home, &file, 5).expect("export");
        let imported: ProfileId = serde_json::from_value(
            Profiles::import(&s, file.to_str().expect("path")).expect("import")["id"].clone(),
        )
        .expect("an id");
        let c = rusqlite::Connection::open(s.paths.profile_db(imported)).expect("open");
        let text: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM page_text WHERE page_text MATCH 'secret'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(text, 0, "page text stays behind");
        let kept: String = c
            .query_row("SELECT title FROM bookmarks WHERE id = 'b1'", [], |r| {
                r.get(0)
            })
            .expect("kept");
        assert_eq!(kept, "Kept");
        let names: Vec<String> = profiles::list(&s.app_db)
            .expect("list")
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["Personal".to_owned(), "Personal".to_owned()]);
        assert!(Profiles::import(&s, dir.path().join("none").to_str().expect("path")).is_err());
    }

    #[test]
    fn rules_match_hosts_and_path_prefixes() {
        use crate::profiles::rule_matches;
        assert!(rule_matches("*.acme.com", "https://docs.ACME.com/x"));
        assert!(rule_matches("*.acme.com", "https://acme.com/"));
        assert!(!rule_matches("*.acme.com", "https://notacme.com/"));
        assert!(rule_matches(
            "github.com/acme/",
            "https://github.com/acme/repo"
        ));
        assert!(!rule_matches(
            "github.com/acme/",
            "https://github.com/other/repo"
        ));
        assert!(!rule_matches(
            "github.com/acme/",
            "https://github.com/ACME/repo"
        ));
    }
}
