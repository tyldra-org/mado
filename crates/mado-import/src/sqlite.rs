//! Other browsers' SQLite files, copied first: the browser may hold locks,
//! and its write-ahead log holds the newest rows (spec 12 §5.2).

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::ImportError;

/// A private copy of `path` with its `-wal` file, open for reading. The
/// temporary folder goes with the returned guard.
pub fn open_copy(path: &Path) -> Result<(Connection, tempfile::TempDir), ImportError> {
    let dir = tempfile::tempdir().map_err(|e| ImportError::io(path, e))?;
    let name = path
        .file_name()
        .ok_or_else(|| ImportError::format(path, "no file name"))?;
    let copy = dir.path().join(name);
    std::fs::copy(path, &copy).map_err(|e| ImportError::io(path, e))?;
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let wal = std::path::PathBuf::from(wal);
    if wal.exists() {
        let mut to = copy.as_os_str().to_owned();
        to.push("-wal");
        std::fs::copy(&wal, std::path::PathBuf::from(to)).map_err(|e| ImportError::io(&wal, e))?;
    }
    // Read-write on the copy, so SQLite folds the copied log in.
    let conn = Connection::open_with_flags(
        &copy,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| ImportError::Sqlite(path.to_owned(), e))?;
    Ok((conn, dir))
}
