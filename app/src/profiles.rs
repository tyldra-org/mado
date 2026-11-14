//! Choosing the profile a launch runs (spec 01 §1.1 step 2) from the app
//! database (spec 12 §1.1).

use mado_storage::Storage;
use mado_types::ProfileId;
use mado_types::state::ProfileInfo;
use rusqlite::{OptionalExtension, params};

use crate::args::Args;

/// A row of `profiles`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub color: String,
    pub avatar: String,
}

impl Profile {
    pub fn info(&self) -> ProfileInfo {
        ProfileInfo {
            id: self.id,
            name: self.name.clone(),
            color: self.color.clone(),
            avatar: self.avatar.clone(),
            is_private: false,
            is_guest: false,
        }
    }

    /// The Dock badge: the name's first letter.
    pub fn initial(&self) -> String {
        self.name
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Choice {
    Run(Profile),
    /// Several profiles, `app.showProfilePicker` on, a Dock launch: ask.
    Picker(Vec<Profile>),
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("no profile called {0}")]
    NotFound(String),
    #[error("app database: {0}")]
    Storage(#[from] mado_storage::Error),
}

const SCHEMA_VERSION: i64 = 1;

pub fn list(app: &Storage) -> Result<Vec<Profile>, ProfileError> {
    Ok(app.readers().read(|c| {
        let mut stmt =
            c.prepare("SELECT id, name, color, avatar FROM profiles ORDER BY sort, created_at")?;
        stmt.query_map([], |r| {
            let id: String = r.get(0)?;
            Ok((id, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .filter_map(|row| match row {
            Ok((id, name, color, avatar)) => id.parse().ok().map(|id| {
                Ok(Profile {
                    id,
                    name,
                    color,
                    avatar,
                })
            }),
            Err(e) => Some(Err(e)),
        })
        .collect()
    })?)
}

/// The first launch makes one profile.
pub fn ensure_one(app: &Storage, now: i64) -> Result<Vec<Profile>, ProfileError> {
    let existing = list(app)?;
    if !existing.is_empty() {
        return Ok(existing);
    }
    let id = ProfileId::new();
    let text = id.to_string();
    app.writer()
        .write(move |c| {
            c.execute(
                "INSERT INTO profiles (id, name, color, avatar, sort, created_at, schema_version) VALUES (?1, 'Personal', 'blue', '', 0, ?2, ?3)",
                params![text, now, SCHEMA_VERSION],
            )
        })
        .wait()?;
    list(app)
}

fn setting_bool(app: &Storage, key: &str, default: bool) -> bool {
    let key = key.to_owned();
    app.readers()
        .read(move |c| {
            c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get::<_, String>(0)
            })
            .optional()
        })
        .ok()
        .flatten()
        .map_or(default, |v| v == "true")
}

fn app_state(app: &Storage, key: &str) -> Option<String> {
    let key = key.to_owned();
    app.readers()
        .read(move |c| {
            c.query_row("SELECT v FROM app_state WHERE k = ?1", [key], |r| r.get(0))
                .optional()
        })
        .ok()
        .flatten()
}

pub fn set_app_state(app: &Storage, key: &str, value: &str) -> mado_storage::PendingWrite<usize> {
    let (key, value) = (key.to_owned(), value.to_owned());
    app.writer().write(move |c| {
        c.execute("INSERT INTO app_state (k, v) VALUES (?1, ?2) ON CONFLICT (k) DO UPDATE SET v = excluded.v", params![key, value])
    })
}

/// A link rule pattern: a host, or `*.` and a domain that matches it and
/// its subdomains (spec 12 §1.3).
fn rule_matches(pattern: &str, url: &str) -> bool {
    let host = url
        .split_once("://")
        .map_or("", |(_, rest)| rest)
        .split(['/', '?', '#', ':'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    match pattern.strip_prefix("*.") {
        Some(domain) => host == domain || host.ends_with(&format!(".{domain}")),
        None => host == pattern,
    }
}

/// Resolves the profile in the order of spec 01 §1.1 step 2.
pub fn choose(app: &Storage, args: &Args, profiles: &[Profile]) -> Result<Choice, ProfileError> {
    if let Some(wanted) = &args.profile {
        return profiles
            .iter()
            .find(|p| p.id.to_string() == *wanted || p.name.eq_ignore_ascii_case(wanted))
            .cloned()
            .map(Choice::Run)
            .ok_or_else(|| ProfileError::NotFound(wanted.clone()));
    }
    if let Some(url) = args.urls.first() {
        let rules: Vec<(String, String)> = app.readers().read(|c| {
            let mut stmt = c.prepare("SELECT pattern, profile_id FROM link_rules ORDER BY sort")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })?;
        if let Some((_, id)) = rules.iter().find(|(pattern, _)| rule_matches(pattern, url))
            && let Some(p) = profiles.iter().find(|p| p.id.to_string() == *id)
        {
            return Ok(Choice::Run(p.clone()));
        }
    }
    let dock = args.urls.is_empty() && args.files.is_empty() && !args.hidden;
    if profiles.len() > 1 && dock && setting_bool(app, "app.showProfilePicker", true) {
        return Ok(Choice::Picker(profiles.to_vec()));
    }
    let last = app_state(app, "last_profile_id");
    Ok(Choice::Run(
        profiles
            .iter()
            .find(|p| Some(p.id.to_string()) == last)
            .or_else(|| profiles.first())
            .cloned()
            .ok_or_else(|| ProfileError::NotFound("any".into()))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mado_storage::{Kind, open};
    use std::ffi::OsString;

    fn app() -> (tempfile::TempDir, Storage) {
        let dir = tempfile::tempdir().expect("dir");
        let opened = open(
            &dir.path().join("app.sqlite"),
            Kind::App,
            &dir.path().join("B"),
            &dir.path().join("X"),
        )
        .expect("opens");
        let storage = Storage::start(&opened).expect("starts");
        (dir, storage)
    }

    fn args(a: &[&str]) -> Args {
        Args::parse(a.iter().map(OsString::from)).expect("parses")
    }

    #[test]
    fn the_first_launch_makes_one_profile_and_runs_it() {
        let (_d, app) = app();
        let profiles = ensure_one(&app, 1).expect("creates");
        assert_eq!(profiles.len(), 1);
        assert_eq!(ensure_one(&app, 2).expect("keeps").len(), 1);
        assert!(
            matches!(choose(&app, &args(&[]), &profiles), Ok(Choice::Run(p)) if p.name == "Personal")
        );
    }

    #[test]
    fn resolution_follows_spec_order() {
        let (_d, app) = app();
        let mut profiles = ensure_one(&app, 1).expect("creates");
        let work = Profile {
            id: ProfileId::new(),
            name: "Work".into(),
            color: "red".into(),
            avatar: String::new(),
        };
        let id = work.id.to_string();
        app.writer()
            .write(move |c| {
                c.execute("INSERT INTO profiles (id, name, color, avatar, sort, created_at, schema_version) VALUES (?1, 'Work', 'red', '', 1, 2, 1)", [&id])?;
                c.execute("INSERT INTO link_rules (id, pattern, profile_id, sort) VALUES ('r1', '*.corp.test', ?1, 0)", [&id])
            })
            .wait()
            .expect("writes");
        profiles.push(work.clone());
        // Explicit, by name or ID.
        assert_eq!(
            choose(&app, &args(&["--profile", "work"]), &profiles).expect("ok"),
            Choice::Run(work.clone())
        );
        assert!(choose(&app, &args(&["--profile", "nobody"]), &profiles).is_err());
        // A link rule.
        assert_eq!(
            choose(&app, &args(&["https://wiki.corp.test/x"]), &profiles).expect("ok"),
            Choice::Run(work.clone())
        );
        // Two profiles and a Dock launch: the picker.
        assert!(matches!(
            choose(&app, &args(&[]), &profiles),
            Ok(Choice::Picker(_))
        ));
        // A link without a rule: the last used profile.
        set_app_state(&app, "last_profile_id", &work.id.to_string())
            .wait()
            .expect("writes");
        assert_eq!(
            choose(&app, &args(&["https://example.com/"]), &profiles).expect("ok"),
            Choice::Run(work)
        );
    }
}
