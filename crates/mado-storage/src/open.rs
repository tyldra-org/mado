//! Opening a database (spec 03 §2, §5): check it, refuse a newer schema
//! without writing, back up before migrating, recover from a backup when the
//! file is damaged.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::backup::{BackupReason, backup, list};
use crate::migrate::{self, Kind, latest_version, user_version};
use crate::time::{now_ms, stamp};
use crate::{Error, PRAGMAS};

/// A checked, migrated database, ready for [`crate::Storage::start`].
#[derive(Debug)]
pub struct Opened {
    pub path: PathBuf,
    pub kind: Kind,
    /// Set when the file was damaged; the core shows the matching notice.
    pub recovery: Option<Recovery>,
}

/// What recovery did with a damaged file. The broken file is kept in
/// `broken` either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// "Mado restored your profile from a backup made on <date>".
    FromBackup {
        backup: PathBuf,
        made_at: i64,
        broken: PathBuf,
    },
    /// "Mado couldn't read your profile data": no backup passed the check.
    Empty { broken: PathBuf },
}

fn read_only(path: &Path) -> Result<Connection, Error> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?)
}

/// `PRAGMA quick_check` passes. Errors that mean "not a database" count as
/// a failed check, not as an error.
fn healthy(path: &Path) -> Result<bool, Error> {
    let check = read_only(path).and_then(|conn| {
        let result: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        Ok(result == "ok")
    });
    match check {
        Ok(ok) => Ok(ok),
        Err(e) if e.is_corruption() => Ok(false),
        Err(e) => Err(e),
    }
}

/// Opens `path` as a `kind` database. `backups` and `broken` are the
/// profile's `Backups/` and `Broken/` folders.
pub fn open(path: &Path, kind: Kind, backups: &Path, broken: &Path) -> Result<Opened, Error> {
    let mut recovery = None;
    if path.exists() {
        if !healthy(path)? {
            recovery = Some(recover(path, kind, backups, broken)?);
        }
        if path.exists() {
            let found = user_version(&read_only(path)?)?;
            let known = latest_version(kind);
            if found > known {
                return Err(Error::NewerVersion {
                    path: path.to_owned(),
                    found,
                    known,
                });
            }
            if found < known && found > 0 {
                backup(
                    path,
                    kind,
                    backups,
                    BackupReason::PreMigration { to: known },
                )?;
            }
        }
    } else if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut conn = Connection::open(path)?;
    conn.execute_batch(PRAGMAS)?;
    migrate::run(&mut conn, kind)?;
    Ok(Opened {
        path: path.to_owned(),
        kind,
        recovery,
    })
}

/// Moves the damaged file aside and restores the newest backup that passes
/// the check, or leaves no file so `open` creates an empty database.
fn recover(path: &Path, kind: Kind, backups: &Path, broken_dir: &Path) -> Result<Recovery, Error> {
    std::fs::create_dir_all(broken_dir)?;
    let broken = broken_dir.join(format!("{}-{}.sqlite", kind.stem(), stamp(now_ms())));
    std::fs::rename(path, &broken)?;
    for side in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(side);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
    for candidate in list(kind, backups) {
        if healthy(&candidate)? {
            std::fs::copy(&candidate, path)?;
            let made_at = std::fs::metadata(&candidate)?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
                .unwrap_or(0);
            return Ok(Recovery::FromBackup {
                backup: candidate,
                made_at,
                broken,
            });
        }
    }
    Ok(Recovery::Empty { broken })
}
