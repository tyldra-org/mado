//! `vault.sqlite` (spec 11 §2.2): setup, unlock, recovery and the
//! encrypted items. Functions take a connection so the core runs them on
//! its storage threads; none of them keeps a key.

use std::path::Path;

use mado_types::state::VaultStatus;
use rusqlite::{Connection, OptionalExtension, params};
use zeroize::Zeroizing;

use crate::crypto::{open, seal};
use crate::item::{Item, ItemKind};
use crate::wrap::{self, Enclave};
use crate::{Keys, VaultError, VaultKey, recovery};

/// Recently deleted items are purged after 30 days.
pub const PURGE_AFTER_MS: i64 = 30 * 24 * 3600 * 1000;
const RECOVERY_AAD: &[u8] = b"mado-vault-recovery-v1";

/// An item as stored, decrypted.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub id: String,
    pub item: Item,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_used_at: Option<i64>,
    pub deleted_at: Option<i64>,
}

/// An item in Recently deleted.
pub type DeletedItem = Stored;

fn meta(conn: &Connection, k: &str) -> rusqlite::Result<Option<Vec<u8>>> {
    conn.query_row("SELECT v FROM vault_meta WHERE k = ?1", [k], |r| r.get(0))
        .optional()
}

fn set_meta(conn: &Connection, k: &str, v: &[u8]) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO vault_meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = ?2",
        params![k, v],
    )
    .map(|_| ())
}

/// The vault's operations. `dir` is the profile folder, which holds the
/// wrapped key files.
#[derive(Debug)]
pub struct Vault;

impl Vault {
    /// `none` until set up; otherwise `locked` (the caller knows when it
    /// holds keys).
    pub fn status(conn: &Connection) -> Result<VaultStatus, VaultError> {
        Ok(if meta(conn, "created_at")?.is_some() {
            VaultStatus::Locked
        } else {
            VaultStatus::None
        })
    }

    /// Turns Mado Passwords on: a new `VK` wrapped by the enclave and by the
    /// recovery secret, which is returned once for the person to keep.
    pub fn setup(
        conn: &Connection,
        enclave: &dyn Enclave,
        dir: &Path,
        now: i64,
    ) -> Result<(Keys, String), VaultError> {
        if meta(conn, "created_at")?.is_some() {
            return Err(VaultError::AlreadySetUp);
        }
        let vk: VaultKey = Zeroizing::new(crate::random());
        wrap::store(enclave, dir, &vk)?;
        let (secret, text) = recovery::generate();
        let salt: [u8; 16] = crate::random();
        let rk = recovery::derive(&secret, &salt)?;
        let (nonce, data) = seal(&rk, vk.as_ref(), RECOVERY_AAD);
        set_meta(conn, "version", b"1")?;
        set_meta(conn, "recovery_salt", &salt)?;
        set_meta(conn, "recovery_wrapped_vk", &[nonce, data].concat())?;
        set_meta(conn, "created_at", now.to_string().as_bytes())?;
        Ok((Keys::derive(&vk), text))
    }

    /// Unlocks with the enclave (Touch ID). Blocks while the sheet shows.
    pub fn unlock(
        conn: &Connection,
        enclave: &dyn Enclave,
        dir: &Path,
        reason: &str,
    ) -> Result<Keys, VaultError> {
        if meta(conn, "created_at")?.is_none() {
            return Err(VaultError::NotSetUp);
        }
        let vk = wrap::load(enclave, dir, reason)?;
        Ok(Keys::derive(&vk))
    }

    /// Opens the vault with the recovery secret, as on a new Mac, and wraps
    /// `VK` again with this Mac's enclave.
    pub fn recover(
        conn: &Connection,
        enclave: &dyn Enclave,
        dir: &Path,
        secret: &str,
    ) -> Result<Keys, VaultError> {
        let secret = recovery::parse(secret)?;
        let (Some(salt), Some(wrapped)) = (
            meta(conn, "recovery_salt")?,
            meta(conn, "recovery_wrapped_vk")?,
        ) else {
            return Err(VaultError::NotSetUp);
        };
        if wrapped.len() < 12 {
            return Err(VaultError::BadKey);
        }
        let rk = recovery::derive(&secret, &salt)?;
        let raw = open(&rk, &wrapped[..12], &wrapped[12..], RECOVERY_AAD)
            .map_err(|_| VaultError::BadRecoverySecret)?;
        let mut vk: VaultKey = Zeroizing::new([0u8; 32]);
        if raw.len() != 32 {
            return Err(VaultError::BadKey);
        }
        vk.copy_from_slice(&raw);
        wrap::remove(dir);
        wrap::store(enclave, dir, &vk)?;
        Ok(Keys::derive(&vk))
    }

    fn write_index(conn: &Connection, keys: &Keys, id: &str, item: &Item) -> rusqlite::Result<()> {
        conn.execute("DELETE FROM item_index WHERE item_id = ?1", [id])?;
        for text in item.index_texts() {
            conn.execute(
                "INSERT OR IGNORE INTO item_index (item_id, tag) VALUES (?1, ?2)",
                params![id, keys.tag(&text)],
            )?;
        }
        Ok(())
    }

    /// Adds an item; returns its ID.
    pub fn add(
        conn: &Connection,
        keys: &Keys,
        item: &Item,
        now: i64,
    ) -> Result<String, VaultError> {
        let id = mado_types::VaultItemId::new().to_string();
        let kind = item.kind().column();
        let json = Zeroizing::new(serde_json::to_vec(item)?);
        let (nonce, data) = keys.seal_item(&id, kind, &json);
        conn.execute(
            "INSERT INTO items (id, kind, nonce, data, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, kind, nonce, data, now],
        )?;
        Self::write_index(conn, keys, &id, item)?;
        Ok(id)
    }

    /// Replaces an item. A login whose password changes keeps the old one
    /// in its password history.
    pub fn update(
        conn: &Connection,
        keys: &Keys,
        id: &str,
        item: &Item,
        now: i64,
    ) -> Result<(), VaultError> {
        let old = Self::get(conn, keys, id)?;
        let mut item = item.clone();
        if let (Item::Login(new), Item::Login(was)) = (&mut item, &old.item)
            && new.password != was.password
            && !was.password.is_empty()
        {
            new.password_history = was.password_history.clone();
            new.password_history.insert(
                0,
                crate::item::PasswordChange {
                    password: was.password.clone(),
                    changed_at: now,
                },
            );
            new.password_history.truncate(20);
        }
        let kind = item.kind().column();
        let json = Zeroizing::new(serde_json::to_vec(&item)?);
        let (nonce, data) = keys.seal_item(id, kind, &json);
        conn.execute(
            "UPDATE items SET kind = ?2, nonce = ?3, data = ?4, updated_at = ?5 WHERE id = ?1",
            params![id, kind, nonce, data, now],
        )?;
        Self::write_index(conn, keys, id, &item)?;
        Ok(())
    }

    fn decode(keys: &Keys, row: &rusqlite::Row<'_>) -> Result<Stored, VaultError> {
        let id: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let nonce: Vec<u8> = row.get(2)?;
        let data: Vec<u8> = row.get(3)?;
        let json = keys.open_item(&id, &kind, &nonce, &data)?;
        let item: Item = serde_json::from_slice(&json)?;
        if item.kind() != ItemKind::from_column(&kind).ok_or(VaultError::BadKey)? {
            return Err(VaultError::BadKey);
        }
        Ok(Stored {
            id,
            item,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
            last_used_at: row.get(6)?,
            deleted_at: row.get(7)?,
        })
    }

    const COLUMNS: &'static str =
        "id, kind, nonce, data, created_at, updated_at, last_used_at, deleted_at";

    pub fn get(conn: &Connection, keys: &Keys, id: &str) -> Result<Stored, VaultError> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM items WHERE id = ?1",
            Self::COLUMNS
        ))?;
        let mut rows = stmt.query([id])?;
        let row = rows.next()?.ok_or(VaultError::NotFound)?;
        Self::decode(keys, row)
    }

    /// Every item not deleted, or with `deleted` only Recently deleted.
    pub fn list(conn: &Connection, keys: &Keys, deleted: bool) -> Result<Vec<Stored>, VaultError> {
        let filter = if deleted { "IS NOT NULL" } else { "IS NULL" };
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM items WHERE deleted_at {filter} ORDER BY updated_at DESC, id",
            Self::COLUMNS
        ))?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(Self::decode(keys, row)?);
        }
        Ok(out)
    }

    /// Items for a site (registrable domain) or a passkey RP, through the
    /// HMAC index; deleted items excluded.
    pub fn find(
        conn: &Connection,
        keys: &Keys,
        index_text: &str,
    ) -> Result<Vec<Stored>, VaultError> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM items WHERE deleted_at IS NULL AND id IN (SELECT item_id FROM item_index WHERE tag = ?1)
             ORDER BY last_used_at DESC, updated_at DESC",
            Self::COLUMNS
        ))?;
        let mut rows = stmt.query([keys.tag(index_text)])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(Self::decode(keys, row)?);
        }
        Ok(out)
    }

    pub fn touch(conn: &Connection, id: &str, now: i64) -> Result<(), VaultError> {
        conn.execute(
            "UPDATE items SET last_used_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    /// Moves an item to Recently deleted.
    pub fn delete(conn: &Connection, id: &str, now: i64) -> Result<(), VaultError> {
        if conn.execute(
            "UPDATE items SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
            params![id, now],
        )? == 0
        {
            return Err(VaultError::NotFound);
        }
        Ok(())
    }

    pub fn restore(conn: &Connection, id: &str) -> Result<(), VaultError> {
        if conn.execute(
            "UPDATE items SET deleted_at = NULL WHERE id = ?1 AND deleted_at IS NOT NULL",
            [id],
        )? == 0
        {
            return Err(VaultError::NotFound);
        }
        Ok(())
    }

    pub fn delete_forever(conn: &Connection, id: &str) -> Result<(), VaultError> {
        if conn.execute("DELETE FROM items WHERE id = ?1", [id])? == 0 {
            return Err(VaultError::NotFound);
        }
        Ok(())
    }

    /// Purges items deleted more than 30 days ago; returns how many.
    pub fn purge(conn: &Connection, now: i64) -> Result<usize, VaultError> {
        Ok(conn.execute(
            "DELETE FROM items WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
            [now - PURGE_AFTER_MS],
        )?)
    }

    /// "Never" for a site (registrable domain).
    pub fn never_save(
        conn: &Connection,
        keys: &Keys,
        site: &str,
        never: bool,
    ) -> Result<(), VaultError> {
        let tag = keys.tag(&format!("never:{site}"));
        if never {
            conn.execute("INSERT OR IGNORE INTO never_save (tag) VALUES (?1)", [tag])?;
        } else {
            conn.execute("DELETE FROM never_save WHERE tag = ?1", [tag])?;
        }
        Ok(())
    }

    pub fn is_never_save(conn: &Connection, keys: &Keys, site: &str) -> Result<bool, VaultError> {
        let tag = keys.tag(&format!("never:{site}"));
        Ok(conn
            .query_row("SELECT 1 FROM never_save WHERE tag = ?1", [tag], |_| Ok(()))
            .optional()?
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Card, Login};
    use crate::wrap::SoftEnclave;

    fn db(dir: &Path) -> Connection {
        mado_storage::open(
            &dir.join("vault.sqlite"),
            mado_storage::Kind::Vault,
            &dir.join("Backups"),
            &dir.join("Broken"),
        )
        .expect("open");
        Connection::open(dir.join("vault.sqlite")).expect("conn")
    }

    fn login(site: &str, pw: &str) -> Item {
        Item::Login(Login {
            title: site.into(),
            urls: vec![format!("https://www.{site}/login")],
            username: "me@example.com".into(),
            password: pw.into(),
            ..Login::default()
        })
    }

    #[test]
    fn items_are_encrypted_indexed_and_recently_deleted() {
        let dir = tempfile::tempdir().expect("dir");
        let conn = db(dir.path());
        let mac = SoftEnclave::default();
        assert_eq!(Vault::status(&conn).expect("status"), VaultStatus::None);
        let (keys, _) = Vault::setup(&conn, &mac, dir.path(), 1).expect("setup");
        assert!(matches!(
            Vault::setup(&conn, &mac, dir.path(), 1),
            Err(VaultError::AlreadySetUp)
        ));
        assert_eq!(Vault::status(&conn).expect("status"), VaultStatus::Locked);
        let a = Vault::add(&conn, &keys, &login("example.com", "hunter2"), 10).expect("add");
        Vault::add(&conn, &keys, &login("other.org", "x"), 11).expect("add");
        Vault::add(
            &conn,
            &keys,
            &Item::Card(Card {
                number: "4111111111111111".into(),
                ..Card::default()
            }),
            12,
        )
        .expect("card");
        // Nothing readable on disk.
        let raw: Vec<u8> = std::fs::read(dir.path().join("vault.sqlite")).expect("file");
        let wal = std::fs::read(dir.path().join("vault.sqlite-wal")).unwrap_or_default();
        for needle in [&b"hunter2"[..], b"example.com", b"4111111111111111"] {
            assert!(
                !raw.windows(needle.len()).any(|w| w == needle),
                "plaintext on disk"
            );
            assert!(
                !wal.windows(needle.len()).any(|w| w == needle),
                "plaintext in the log"
            );
        }
        let found = Vault::find(&conn, &keys, "site:example.com").expect("find");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, a);
        // A changed password goes to the history.
        Vault::update(&conn, &keys, &a, &login("example.com", "new-pass"), 20).expect("update");
        let Item::Login(l) = Vault::get(&conn, &keys, &a).expect("get").item else {
            panic!("a login")
        };
        assert_eq!(l.password, "new-pass");
        assert_eq!(l.password_history[0].password, "hunter2");
        // A row copied under another ID does not decrypt.
        conn.execute(
            "INSERT INTO items (id, kind, nonce, data, created_at, updated_at)
             SELECT 'copied', kind, nonce, data, created_at, updated_at FROM items WHERE id = ?1",
            [&a],
        )
        .expect("copy");
        assert!(matches!(
            Vault::get(&conn, &keys, "copied"),
            Err(VaultError::BadKey)
        ));
        Vault::delete_forever(&conn, "copied").expect("gone");
        // Recently deleted, restore, purge after 30 days.
        Vault::delete(&conn, &a, 100).expect("delete");
        assert!(
            Vault::find(&conn, &keys, "site:example.com")
                .expect("find")
                .is_empty()
        );
        assert_eq!(Vault::list(&conn, &keys, true).expect("deleted").len(), 1);
        Vault::restore(&conn, &a).expect("restore");
        Vault::delete(&conn, &a, 100).expect("delete");
        assert_eq!(
            Vault::purge(&conn, 100 + PURGE_AFTER_MS - 1).expect("purge"),
            0
        );
        assert_eq!(
            Vault::purge(&conn, 100 + PURGE_AFTER_MS + 1).expect("purge"),
            1
        );
        assert_eq!(Vault::list(&conn, &keys, false).expect("list").len(), 2);
        Vault::never_save(&conn, &keys, "example.com", true).expect("never");
        assert!(Vault::is_never_save(&conn, &keys, "example.com").expect("never"));
        assert!(!Vault::is_never_save(&conn, &keys, "other.org").expect("never"));
    }

    /// 11 §12 check 7: a vault copied to another computer cannot be opened
    /// without the recovery secret.
    #[test]
    fn a_copied_vault_opens_only_with_the_recovery_secret() {
        let home = tempfile::tempdir().expect("dir");
        let conn = db(home.path());
        let mac = SoftEnclave::default();
        let (keys, secret) = Vault::setup(&conn, &mac, home.path(), 1).expect("setup");
        let id = Vault::add(&conn, &keys, &login("example.com", "pw"), 2).expect("add");
        drop(conn);
        let away = tempfile::tempdir().expect("dir");
        for f in [
            "vault.sqlite",
            "vault.sqlite-wal",
            "vault.se-key",
            "vault.eph-pub",
            "vault.wrapped",
        ] {
            let _ = std::fs::copy(home.path().join(f), away.path().join(f));
        }
        let conn = Connection::open(away.path().join("vault.sqlite")).expect("open");
        let other_mac = SoftEnclave::default();
        assert!(matches!(
            Vault::unlock(&conn, &other_mac, away.path(), "test"),
            Err(VaultError::BadKey)
        ));
        assert!(matches!(
            Vault::recover(
                &conn,
                &other_mac,
                away.path(),
                "AAAAA-AAAAA-AAAAA-AAAAA-AAAAA-A"
            ),
            Err(VaultError::BadRecoverySecret)
        ));
        let keys = Vault::recover(&conn, &other_mac, away.path(), &secret).expect("recovered");
        assert!(matches!(
            Vault::get(&conn, &keys, &id).expect("item").item,
            Item::Login(_)
        ));
        // The new Mac's enclave now opens it.
        Vault::unlock(&conn, &other_mac, away.path(), "test").expect("unlocks after recovery");
    }
}
