//! Spec 03 §8 checks 1, 2 and 4, and the writer, pool, backups, change log
//! and limits behind them.

use std::path::{Path, PathBuf};

use mado_storage::{
    BackupReason, Error, Kind, Recovery, Storage, backup, latest_version, limits, open,
    prune_backups,
};
use rusqlite::Connection;

struct Profile {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Profile {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().to_owned();
        Self { _dir: dir, root }
    }
    fn db(&self) -> PathBuf {
        self.root.join("profile.sqlite")
    }
    fn backups(&self) -> PathBuf {
        self.root.join("Backups")
    }
    fn broken(&self) -> PathBuf {
        self.root.join("Broken")
    }
    fn open(&self, kind: Kind) -> Result<mado_storage::Opened, Error> {
        open(&self.db(), kind, &self.backups(), &self.broken())
    }
}

fn tables(path: &Path) -> Vec<String> {
    let conn = Connection::open(path).expect("open");
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type IN ('table', 'trigger') ORDER BY name")
        .expect("prepare");
    stmt.query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

fn version(path: &Path) -> i64 {
    Connection::open(path)
        .and_then(|c| c.pragma_query_value(None, "user_version", |r| r.get(0)))
        .expect("version")
}

#[test]
fn check_4_every_database_migrates_from_version_0() {
    for kind in [Kind::App, Kind::Profile, Kind::Vault] {
        let p = Profile::new();
        let opened = p.open(kind).expect("opens");
        assert_eq!(opened.recovery, None);
        assert_eq!(version(&p.db()), latest_version(kind));
        let names = tables(&p.db());
        let expected: &[&str] = match kind {
            Kind::App => &["app_state", "link_rules", "profiles", "settings"],
            Kind::Profile => &[
                "windows",
                "tabs",
                "groups",
                "bookmarks",
                "settings",
                "settings_log",
                "change_log",
                "page_text",
                "change_log_settings_update",
            ],
            Kind::Vault => &["vault_meta", "items", "item_index", "never_save"],
        };
        for name in expected {
            assert!(names.iter().any(|n| n == name), "{kind:?} lacks {name}");
        }
        // Opening again is a no-op.
        p.open(kind).expect("reopens");
        assert_eq!(version(&p.db()), latest_version(kind));
    }
}

#[test]
fn check_1_a_newer_schema_is_refused_without_writing() {
    let p = Profile::new();
    p.open(Kind::Profile).expect("opens");
    Connection::open(p.db())
        .and_then(|c| c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA user_version = 99;"))
        .expect("bump");
    let before = std::fs::read(p.db()).expect("read");
    let err = p.open(Kind::Profile).expect_err("refuses");
    assert!(
        matches!(err, Error::NewerVersion { found: 99, .. }),
        "{err}"
    );
    assert_eq!(std::fs::read(p.db()).expect("read"), before);
    assert!(!p.backups().exists(), "no backup either");
}

#[test]
fn check_2_a_corrupted_profile_is_restored_from_the_newest_backup() {
    let p = Profile::new();
    let opened = p.open(Kind::Profile).expect("opens");
    {
        let storage = Storage::start(&opened).expect("starts");
        storage
            .writer()
            .write(|c| {
                c.execute(
                    "INSERT INTO app_link_rules_probe_does_not_exist VALUES (1)",
                    [],
                )
            })
            .wait()
            .expect_err("bad SQL fails alone");
        storage
            .writer()
            .write(|c| {
                c.execute(
                    "INSERT INTO bookmarks (id, kind, title, sort, created_at, updated_at) VALUES ('b1', 'link', 'kept', 0, 1, 1)",
                    [],
                )
            })
            .wait()
            .expect("writes");
    }
    Connection::open(p.db())
        .and_then(|c| c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);"))
        .expect("checkpoint");
    backup(&p.db(), Kind::Profile, &p.backups(), BackupReason::Daily).expect("backs up");

    // Overwrite bytes in the middle of the file.
    let mut bytes = std::fs::read(p.db()).expect("read");
    let mid = bytes.len() / 2;
    for b in &mut bytes[mid - 2048..mid + 2048] {
        *b = 0xA5;
    }
    bytes[..16].copy_from_slice(b"definitely bad!\0");
    std::fs::write(p.db(), &bytes).expect("corrupt");

    let opened = p.open(Kind::Profile).expect("recovers");
    let Some(Recovery::FromBackup { broken, .. }) = &opened.recovery else {
        panic!("expected a restore, got {:?}", opened.recovery);
    };
    assert!(broken.exists(), "the broken file is kept");
    let title: String = Connection::open(p.db())
        .and_then(|c| {
            c.query_row("SELECT title FROM bookmarks WHERE id = 'b1'", [], |r| {
                r.get(0)
            })
        })
        .expect("restored row");
    assert_eq!(title, "kept");
}

#[test]
fn a_damaged_file_without_backups_becomes_an_empty_database() {
    let p = Profile::new();
    std::fs::write(
        p.db(),
        b"not a database at all, just bytes that go on and on",
    )
    .expect("write");
    let opened = p.open(Kind::Profile).expect("opens");
    assert!(matches!(opened.recovery, Some(Recovery::Empty { .. })));
    assert_eq!(version(&p.db()), latest_version(Kind::Profile));
}

#[test]
fn a_file_at_version_0_is_migrated_without_a_backup() {
    // Version 1 is the only migration today. A file at user_version 0 holds
    // no Mado data, so there is nothing to back up; from version 2 on, an
    // older file is copied to Backups/ as profile-<stamp>-pre-v<N>.sqlite.
    let p = Profile::new();
    Connection::open(p.db())
        .and_then(|c| c.execute_batch("CREATE TABLE legacy (x); PRAGMA user_version = 0;"))
        .expect("seed");
    p.open(Kind::Profile).expect("migrates");
    // user_version 0 means a fresh file: nothing to protect, no backup.
    assert!(!p.backups().exists());
}

#[test]
fn the_writer_reports_a_write_only_after_its_batch_commits_and_isolates_failures() {
    let p = Profile::new();
    let storage = Storage::start(&p.open(Kind::Profile).expect("opens")).expect("starts");
    let pending: Vec<_> = (0..200)
        .map(|i| {
            storage.writer().write(move |c| {
                if i % 50 == 7 {
                    c.execute("INSERT INTO nope VALUES (1)", [])?;
                }
                c.execute(
                    "INSERT INTO closed_stack (kind, payload, closed_at) VALUES ('tab', ?1, ?2)",
                    rusqlite::params![format!("{{\"i\":{i}}}"), i],
                )
            })
        })
        .collect();
    let failed = pending
        .into_iter()
        .map(|w| w.wait())
        .filter(Result::is_err)
        .count();
    assert_eq!(failed, 4);
    let rows: i64 = storage
        .readers()
        .read(|c| c.query_row("SELECT COUNT(*) FROM closed_stack", [], |r| r.get(0)))
        .expect("reads");
    assert_eq!(rows, 196);
    let trimmed = storage
        .writer()
        .write(limits::trim_closed_stack)
        .wait()
        .expect("trims");
    assert_eq!(trimmed, 146);
}

#[test]
fn readers_cannot_write() {
    let p = Profile::new();
    let storage = Storage::start(&p.open(Kind::Profile).expect("opens")).expect("starts");
    let err = storage
        .readers()
        .read(|c| c.execute("DELETE FROM tabs", []))
        .expect_err("read-only");
    assert!(err.to_string().contains("readonly"), "{err}");
}

#[test]
fn synced_tables_append_to_the_change_log() {
    let p = Profile::new();
    let storage = Storage::start(&p.open(Kind::Profile).expect("opens")).expect("starts");
    storage
        .writer()
        .write(|c| {
            c.execute("INSERT INTO settings (key, value, schema_version, updated_at) VALUES ('a.b', '1', 1, 1)", [])?;
            c.execute("UPDATE settings SET value = '2' WHERE key = 'a.b'", [])?;
            c.execute(
                "INSERT INTO site_settings (origin, key, value, granted_at, source) VALUES ('https://x.test', 'camera', 'allow', 1, 'user')",
                [],
            )?;
            c.execute("DELETE FROM settings WHERE key = 'a.b'", [])
        })
        .wait()
        .expect("writes");
    let log: Vec<(String, String, String)> = storage
        .readers()
        .read(|c| {
            let mut s = c.prepare("SELECT tbl, row_id, op FROM change_log ORDER BY seq")?;
            s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect()
        })
        .expect("reads");
    assert_eq!(
        log,
        vec![
            ("settings".into(), "a.b".into(), "insert".into()),
            ("settings".into(), "a.b".into(), "update".into()),
            (
                "site_settings".into(),
                "https://x.test|camera|".into(),
                "insert".into()
            ),
            ("settings".into(), "a.b".into(), "delete".into()),
        ]
    );
    // Not old enough to trim.
    let now = i64::MAX / 2;
    assert_eq!(
        storage
            .writer()
            .write(move |c| limits::trim_change_log(c, 0))
            .wait()
            .expect("trims"),
        0
    );
    assert_eq!(
        storage
            .writer()
            .write(move |c| limits::trim_change_log(c, now))
            .wait()
            .expect("trims"),
        4
    );
}

#[test]
fn backups_keep_seven_daily_copies() {
    let p = Profile::new();
    p.open(Kind::Profile).expect("opens");
    std::fs::create_dir_all(p.backups()).expect("dir");
    for day in 1..=9 {
        std::fs::copy(
            p.db(),
            p.backups()
                .join(format!("profile-202611{day:02}-030000.sqlite")),
        )
        .expect("copy");
    }
    std::fs::copy(
        p.db(),
        p.backups().join("profile-20261101-020000-pre-v2.sqlite"),
    )
    .expect("copy");
    let deleted = prune_backups(Kind::Profile, &p.backups()).expect("prunes");
    let names: Vec<String> = deleted
        .iter()
        .filter_map(|d| d.file_name()?.to_str().map(str::to_owned))
        .collect();
    assert_eq!(
        names,
        [
            "profile-20261102-030000.sqlite",
            "profile-20261101-030000.sqlite"
        ]
    );
    assert!(
        p.backups()
            .join("profile-20261101-020000-pre-v2.sqlite")
            .exists(),
        "recent pre-migration copy kept"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn writes_can_be_awaited() {
    let p = Profile::new();
    let storage = Storage::start(&p.open(Kind::App).expect("opens")).expect("starts");
    let n = storage
        .writer()
        .write(|c| {
            c.execute(
                "INSERT INTO app_state (k, v) VALUES ('clean_quit', '1')",
                [],
            )
        })
        .await
        .expect("writes");
    assert_eq!(n, 1);
}
