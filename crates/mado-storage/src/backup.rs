//! Backups (spec 03 §4): `VACUUM INTO` copies in the database's
//! `Backups/` folder. Daily copies keep the newest 7; pre-migration copies
//! keep 30 days.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::time::{now_ms, stamp};
use crate::{Error, Kind};

/// Why a backup was made; it decides how long the copy is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupReason {
    Daily,
    /// Before migrating to `to`.
    PreMigration {
        to: i64,
    },
}

const DAILY_KEEP: usize = 7;
const PRE_MIGRATION_KEEP_MS: i64 = 30 * 24 * 3600 * 1000;

/// Copies the database at `db` into `dir` and returns the copy's path. Runs
/// on a read-only connection, so the original is never written. The vault
/// copy stays encrypted: its items are encrypted row by row (spec 11).
pub fn backup(db: &Path, kind: Kind, dir: &Path, reason: BackupReason) -> Result<PathBuf, Error> {
    std::fs::create_dir_all(dir)?;
    let suffix = match reason {
        BackupReason::Daily => String::new(),
        BackupReason::PreMigration { to } => format!("-pre-v{to}"),
    };
    let target = dir.join(format!(
        "{}-{}{suffix}.sqlite",
        kind.stem(),
        stamp(now_ms())
    ));
    let _ = std::fs::remove_file(&target);
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.execute("VACUUM INTO ?1", [target.to_string_lossy()])?;
    Ok(target)
}

/// This kind's backups in `dir`, newest first (names sort by time).
pub(crate) fn list(kind: Kind, dir: &Path) -> Vec<PathBuf> {
    let prefix = format!("{}-", kind.stem());
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".sqlite"))
        })
        .collect();
    files.sort();
    files.reverse();
    files
}

/// Deletes backups past their retention: daily copies beyond the newest 7,
/// pre-migration copies older than 30 days. Returns what it deleted.
pub fn prune_backups(kind: Kind, dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let now = now_ms();
    let mut daily = 0;
    let mut deleted = Vec::new();
    for file in list(kind, dir) {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let expired = if name.contains("-pre-v") {
            let modified = std::fs::metadata(&file)?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
                .unwrap_or(0);
            now - modified > PRE_MIGRATION_KEEP_MS
        } else {
            daily += 1;
            daily > DAILY_KEEP
        };
        if expired {
            std::fs::remove_file(&file)?;
            deleted.push(file);
        }
    }
    Ok(deleted)
}
