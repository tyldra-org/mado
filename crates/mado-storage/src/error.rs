use std::path::PathBuf;

/// What can go wrong opening or using a database.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    /// A newer Mado used this database. Nothing was written (spec 03 §2).
    #[error("{path} has schema version {found}; this build knows up to {known}")]
    NewerVersion {
        path: PathBuf,
        found: i64,
        known: i64,
    },
    /// The batch holding this write did not commit; the write is undone.
    #[error("commit failed: {0}")]
    CommitFailed(String),
    /// The writer thread has stopped, or could not start a transaction.
    #[error("the database writer has stopped")]
    WriterStopped,
}

impl Error {
    /// True for the errors that mean the file itself is damaged.
    pub fn is_corruption(&self) -> bool {
        matches!(
            self,
            Self::Sqlite(rusqlite::Error::SqliteFailure(e, _))
                if matches!(e.code, rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
        )
    }
}
