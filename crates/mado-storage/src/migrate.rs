//! Ordered, embedded migrations per database (spec 03 §2).

use rusqlite::{Connection, TransactionBehavior};

use crate::Error;

/// Which database a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// `<root>/app.sqlite`, shared by every profile process.
    App,
    /// `<profile>/profile.sqlite`.
    Profile,
    /// `<profile>/vault.sqlite`.
    Vault,
}

impl Kind {
    /// The file stem, used for backup and broken-file names.
    pub fn stem(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Profile => "profile",
            Self::Vault => "vault",
        }
    }

    /// `(version, sql)` in order; version `n` is the n-th entry.
    pub(crate) fn migrations(self) -> &'static [&'static str] {
        match self {
            Self::App => &[include_str!("../migrations/app/0001_initial.sql")],
            Self::Profile => &[include_str!("../migrations/profile/0001_initial.sql")],
            Self::Vault => &[include_str!("../migrations/vault/0001_initial.sql")],
        }
    }
}

/// The newest schema version this build knows for `kind`.
pub fn latest_version(kind: Kind) -> i64 {
    i64::try_from(kind.migrations().len()).unwrap_or(i64::MAX)
}

pub(crate) fn user_version(conn: &Connection) -> Result<i64, Error> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

/// Runs each pending migration in its own transaction.
pub(crate) fn run(conn: &mut Connection, kind: Kind) -> Result<(), Error> {
    let current = user_version(conn)?;
    for (index, sql) in kind.migrations().iter().enumerate() {
        let version = i64::try_from(index).unwrap_or(i64::MAX) + 1;
        if version <= current {
            continue;
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;
    }
    Ok(())
}
