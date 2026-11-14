//! Size limits (spec 03 §6, §7) as functions over one transaction. The core
//! schedules them (daily, and after writes that grow a table).

use rusqlite::{Connection, params};

/// Most recent closed tabs, groups and windows kept for ⌘⇧T.
pub const CLOSED_STACK: i64 = 50;
/// Settings changes kept for undo and the log.
pub const SETTINGS_LOG: i64 = 500;
/// Change log entries kept while sync does not exist.
pub const CHANGE_LOG_DAYS: i64 = 30;
/// Page text index stops at this share of its limit when trimmed.
const PAGE_TEXT_TARGET: f64 = 0.9;

const DAY_MS: i64 = 24 * 3600 * 1000;

/// Keeps the newest `CLOSED_STACK` entries. Returns rows deleted.
pub fn trim_closed_stack(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM closed_stack WHERE id NOT IN (SELECT id FROM closed_stack ORDER BY id DESC LIMIT ?1)",
        [CLOSED_STACK],
    )
}

/// Keeps the newest `SETTINGS_LOG` entries.
pub fn trim_settings_log(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM settings_log WHERE id NOT IN (SELECT id FROM settings_log ORDER BY id DESC LIMIT ?1)",
        [SETTINGS_LOG],
    )
}

/// Drops change log entries older than 30 days.
pub fn trim_change_log(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM change_log WHERE at < ?1",
        [now - CHANGE_LOG_DAYS * DAY_MS],
    )
}

/// Deletes visits older than `retention_days` (setting `history.retention`)
/// and URLs left without visits. Returns visits deleted.
pub fn purge_history(conn: &Connection, now: i64, retention_days: i64) -> rusqlite::Result<usize> {
    let visits = conn.execute(
        "DELETE FROM visits WHERE visited_at < ?1",
        [now - retention_days * DAY_MS],
    )?;
    conn.execute(
        "DELETE FROM urls WHERE id NOT IN (SELECT DISTINCT url_id FROM visits)",
        [],
    )?;
    Ok(visits)
}

/// While the page text index exceeds `limit_bytes` (setting
/// `history.indexLimitGb`), deletes the oldest indexed pages until it is
/// under 90% of the limit. Returns pages deleted.
pub fn trim_page_text(conn: &Connection, limit_bytes: i64) -> rusqlite::Result<usize> {
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(bytes), 0) FROM page_text_meta",
        [],
        |r| r.get(0),
    )?;
    if total <= limit_bytes {
        return Ok(0);
    }
    // Truncation toward zero is fine for a byte target.
    #[expect(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    let target = (limit_bytes as f64 * PAGE_TEXT_TARGET) as i64;
    let mut stmt =
        conn.prepare("SELECT url_id, bytes FROM page_text_meta ORDER BY indexed_at ASC")?;
    let rows: Vec<(i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut remaining = total;
    let mut deleted = 0;
    for (url_id, bytes) in rows {
        if remaining <= target {
            break;
        }
        conn.execute("DELETE FROM page_text WHERE rowid = ?1", params![url_id])?;
        conn.execute(
            "DELETE FROM page_text_meta WHERE url_id = ?1",
            params![url_id],
        )?;
        remaining -= bytes;
        deleted += 1;
    }
    Ok(deleted)
}
