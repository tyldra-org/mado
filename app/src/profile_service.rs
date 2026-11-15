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
        let secret = mado_control::LaunchSecret::read(&self.paths.launch_secret(id))
            .map_err(|e| e.to_string())?;
        let request = LaunchRequest {
            urls: Vec::new(),
            files: Vec::new(),
            new_window: false,
            private: false,
            source: LaunchSource::Cli,
            control: Some(action),
        };
        let socket = self.paths.control_socket(id);
        tauri::async_runtime::block_on(mado_control::forward(&socket, &secret, &request))
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
