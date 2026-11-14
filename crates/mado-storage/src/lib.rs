//! SQLite connections, migrations, backups, recovery and the change log.
//!
//! Specification: `mado-docs/spec/03-storage.md`. Roadmap: M1-T04.
//!
//! [`open`] checks, backs up, migrates or recovers a database file and hands
//! back a [`Storage`]: one writer thread that batches writes into
//! transactions, and a pool of up to four read-only connections for queries.

mod backup;
mod error;
mod migrate;
mod open;
mod pool;
mod time;
mod writer;

pub mod limits;

pub use backup::{BackupReason, backup, prune_backups};
pub use error::Error;
pub use migrate::{Kind, latest_version};
pub use open::{Opened, Recovery, open};
pub use pool::ReadPool;
pub use writer::{PendingWrite, Writer};

use std::path::{Path, PathBuf};

/// The connection settings every Mado database uses (spec 03 §1).
pub(crate) const PRAGMAS: &str = "PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;";

/// An open database: its writer thread and its read pool.
#[derive(Debug)]
pub struct Storage {
    path: PathBuf,
    kind: Kind,
    writer: Writer,
    readers: ReadPool,
}

impl Storage {
    /// Starts the writer thread and the read pool on a database [`open`]
    /// has checked and migrated.
    pub fn start(opened: &Opened) -> Result<Self, Error> {
        Ok(Self {
            path: opened.path.clone(),
            kind: opened.kind,
            writer: Writer::start(&opened.path)?,
            readers: ReadPool::new(&opened.path, 4),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn writer(&self) -> &Writer {
        &self.writer
    }

    pub fn readers(&self) -> &ReadPool {
        &self.readers
    }
}
