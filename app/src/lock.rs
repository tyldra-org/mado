//! The profile ownership lock (spec 01 §1.2): an exclusive advisory lock on
//! `<profile>/SingletonLock`, held for the life of the process. The OS
//! releases it when the process dies, so a killed process leaves no stale
//! lock.

use std::fs::{File, TryLockError};
use std::io::Write;
use std::path::Path;

#[derive(Debug)]
pub struct ProfileLock {
    // Held, not read: dropping the file releases the lock.
    _file: File,
}

impl ProfileLock {
    /// `Ok(None)` when another process owns the profile.
    pub fn acquire(path: &Path, version: &str, now: i64) -> std::io::Result<Option<Self>> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        // std's file lock is flock(LOCK_EX | LOCK_NB) on macOS, what spec
        // 01 §1.2 asks of the fs4 crate.
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(e)) => return Err(e),
        }
        // For diagnostics only.
        file.set_len(0)?;
        let info =
            serde_json::json!({ "pid": std::process::id(), "startedAt": now, "version": version });
        file.write_all(info.to_string().as_bytes())?;
        file.flush()?;
        Ok(Some(Self { _file: file }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_owner_at_a_time_and_released_on_drop() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("p/SingletonLock");
        let first = ProfileLock::acquire(&path, "0", 1)
            .expect("io")
            .expect("owner");
        assert!(ProfileLock::acquire(&path, "0", 2).expect("io").is_none());
        drop(first);
        assert!(ProfileLock::acquire(&path, "0", 3).expect("io").is_some());
        assert!(
            std::fs::read_to_string(&path)
                .expect("reads")
                .contains("\"pid\"")
        );
    }
}
