//! Read-only connections for queries (spec 01 §4.3, 03 §1): up to `max`,
//! opened on first use and reused.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

use rusqlite::{Connection, OpenFlags};

use crate::{Error, PRAGMAS};

#[derive(Debug)]
pub struct ReadPool {
    path: PathBuf,
    max: usize,
    state: Mutex<PoolState>,
    freed: Condvar,
}

#[derive(Debug, Default)]
struct PoolState {
    idle: Vec<Connection>,
    open: usize,
}

impl ReadPool {
    pub fn new(path: &Path, max: usize) -> Self {
        Self {
            path: path.to_owned(),
            max: max.max(1),
            state: Mutex::new(PoolState::default()),
            freed: Condvar::new(),
        }
    }

    /// Runs `f` on a read-only connection, waiting while all are busy.
    /// Blocks: call it from the worker pool, never from the core task.
    pub fn read<R>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<R>) -> Result<R, Error> {
        let conn = self.take()?;
        let result = f(&conn).map_err(Error::from);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.idle.push(conn);
        drop(state);
        self.freed.notify_one();
        result
    }

    fn take(&self) -> Result<Connection, Error> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(conn) = state.idle.pop() {
                return Ok(conn);
            }
            if state.open < self.max {
                state.open += 1;
                drop(state);
                return self.connect().inspect_err(|_| {
                    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.open -= 1;
                });
            }
            state = self.freed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn connect(&self) -> Result<Connection, Error> {
        let conn = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // journal_mode needs write access and is already WAL; the rest apply.
        conn.execute_batch(&PRAGMAS.replace("PRAGMA journal_mode = WAL;", ""))?;
        Ok(conn)
    }
}
