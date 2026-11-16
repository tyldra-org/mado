//! History (spec 12 §2, 03 §7): recording visits with their transitions,
//! frecency (spec 05 §2), page text in the full-text index, queries,
//! deletion, exclusions, CSV export and retention. Plain functions over a
//! SQLite connection: the core runs writes on the writer thread and queries
//! on the read pool.

use std::fmt::Write as _;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// How a visit happened (spec 12 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transition {
    Typed,
    Link,
    Bookmark,
    Reload,
    Redirect,
    FormSubmit,
    Restore,
    External,
    /// From another browser's history (spec 12 §5); counted like `link`.
    Imported,
}

impl Transition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::Link => "link",
            Self::Bookmark => "bookmark",
            Self::Reload => "reload",
            Self::Redirect => "redirect",
            Self::FormSubmit => "formSubmit",
            Self::Restore => "restore",
            Self::External => "external",
            Self::Imported => "imported",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "typed" => Self::Typed,
            "bookmark" => Self::Bookmark,
            "reload" => Self::Reload,
            "redirect" => Self::Redirect,
            "formSubmit" => Self::FormSubmit,
            "restore" => Self::Restore,
            "external" => Self::External,
            "imported" => Self::Imported,
            _ => Self::Link,
        }
    }

    /// Spec 05 §2's weights; `restore` is not counted.
    fn weight(self) -> f64 {
        match self {
            Self::Typed => 2.0,
            Self::Bookmark => 1.4,
            Self::Link | Self::External | Self::FormSubmit | Self::Imported => 1.0,
            Self::Reload | Self::Redirect | Self::Restore => 0.0,
        }
    }
}

const DAY: i64 = 86_400_000;

/// Spec 05 §2's age buckets.
fn bucket(age_ms: i64) -> f64 {
    match age_ms / DAY {
        d if d < 4 => 100.0,
        d if d < 14 => 70.0,
        d if d < 31 => 50.0,
        d if d < 90 => 30.0,
        _ => 10.0,
    }
}

/// The URL as history keys it: no fragment.
pub fn key(url: &str) -> &str {
    url.split('#').next().unwrap_or(url)
}

pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_ascii_lowercase())
        })
        .unwrap_or_default()
}

/// Whether a URL is recorded at all (spec 12 §2.1).
pub fn recordable(url: &str) -> bool {
    let scheme = url
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(scheme.as_str(), "http" | "https" | "file")
}

/// Recomputes one URL's frecency from its last 10 visits.
pub fn recompute(conn: &Connection, url_id: i64, now: i64) -> rusqlite::Result<f64> {
    let mut stmt =
        conn.prepare_cached("SELECT transition, visited_at FROM visits WHERE url_id = ?1 ORDER BY visited_at DESC LIMIT 10")?;
    let score: f64 = stmt
        .query_map([url_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .filter_map(Result::ok)
        .map(|(t, at)| Transition::parse(&t).weight() * bucket(now - at))
        .sum();
    conn.execute(
        "UPDATE urls SET frecency = ?2 WHERE id = ?1",
        params![url_id, score],
    )?;
    Ok(score)
}

/// After visits were deleted by time (spec 08 §7): each URL that lost
/// visits gets its counts, last visit and frecency from those left.
pub fn recount(conn: &Connection, now: i64) -> rusqlite::Result<()> {
    let changed: Vec<i64> = conn
        .prepare(
            "SELECT id FROM urls WHERE visit_count != (SELECT COUNT(*) FROM visits WHERE url_id = urls.id)",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in changed {
        conn.execute(
            "UPDATE urls SET
               visit_count = (SELECT COUNT(*) FROM visits WHERE url_id = ?1),
               typed_count = MIN(typed_count, (SELECT COUNT(*) FROM visits WHERE url_id = ?1 AND transition = ?2)),
               last_visit_at = (SELECT MAX(visited_at) FROM visits WHERE url_id = ?1)
             WHERE id = ?1",
            params![id, Transition::Typed.as_str()],
        )?;
        recompute(conn, id, now)?;
    }
    Ok(())
}

/// A recorded visit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recorded {
    pub url_id: i64,
    pub visit_id: i64,
}

/// Records one visit (spec 12 §2.1 steps 1 to 3). `None` when the host
/// is excluded.
pub fn record(
    conn: &Connection,
    url: &str,
    title: &str,
    transition: Transition,
    tab: &str,
    from_visit: Option<i64>,
    now: i64,
) -> rusqlite::Result<Option<Recorded>> {
    let url = key(url);
    let host = host_of(url);
    let excluded: bool = conn
        .query_row(
            "SELECT 1 FROM history_exclusions WHERE host = ?1",
            [&host],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if excluded {
        return Ok(None);
    }
    let typed = i64::from(transition == Transition::Typed);
    conn.execute(
        "INSERT INTO urls (url, host, title, visit_count, typed_count, last_visit_at)
         VALUES (?1, ?2, NULLIF(?3, ''), 1, ?4, ?5)
         ON CONFLICT(url) DO UPDATE SET visit_count = visit_count + 1, typed_count = typed_count + ?4,
             last_visit_at = ?5, title = COALESCE(NULLIF(?3, ''), title), hidden = 0",
        params![url, host, title, typed, now],
    )?;
    let url_id: i64 = conn.query_row("SELECT id FROM urls WHERE url = ?1", [url], |r| r.get(0))?;
    conn.execute(
        "INSERT INTO visits (url_id, visited_at, transition, from_visit_id, tab_id) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![url_id, now, transition.as_str(), from_visit, tab],
    )?;
    let visit_id = conn.last_insert_rowid();
    recompute(conn, url_id, now)?;
    Ok(Some(Recorded { url_id, visit_id }))
}

/// Another browser's visits (spec 12 §5.2): transition `imported`, the
/// URL's counts, last visit and frecency recomputed. A visit already here
/// with the same URL and time is skipped, so importing twice adds nothing.
/// Returns how many visits were added.
pub fn import_visits(
    conn: &Connection,
    visits: &[mado_import::Visit],
    now: i64,
) -> rusqlite::Result<usize> {
    let mut added = 0;
    let mut touched = std::collections::BTreeSet::new();
    for v in visits {
        if !recordable(&v.url) {
            continue;
        }
        let url = key(&v.url);
        let host = host_of(url);
        let excluded = conn
            .query_row(
                "SELECT 1 FROM history_exclusions WHERE host = ?1",
                [&host],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if excluded {
            continue;
        }
        conn.execute(
            "INSERT INTO urls (url, host, title, visit_count, last_visit_at) VALUES (?1, ?2, NULLIF(?3, ''), 0, ?4)
             ON CONFLICT(url) DO UPDATE SET title = COALESCE(title, NULLIF(?3, ''))",
            params![url, host, v.title.chars().take(1024).collect::<String>(), v.at],
        )?;
        let url_id: i64 =
            conn.query_row("SELECT id FROM urls WHERE url = ?1", [url], |r| r.get(0))?;
        let seen = conn
            .query_row(
                "SELECT 1 FROM visits WHERE url_id = ?1 AND visited_at = ?2",
                params![url_id, v.at],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if seen {
            continue;
        }
        conn.execute(
            "INSERT INTO visits (url_id, visited_at, transition) VALUES (?1, ?2, ?3)",
            params![url_id, v.at, Transition::Imported.as_str()],
        )?;
        added += 1;
        touched.insert(url_id);
    }
    for id in touched {
        conn.execute(
            "UPDATE urls SET visit_count = (SELECT COUNT(*) FROM visits WHERE url_id = ?1),
               last_visit_at = (SELECT MAX(visited_at) FROM visits WHERE url_id = ?1) WHERE id = ?1",
            [id],
        )?;
        recompute(conn, id, now)?;
    }
    Ok(added)
}

pub fn set_title(conn: &Connection, url: &str, title: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE urls SET title = ?2 WHERE url = ?1",
        params![key(url), title.chars().take(1024).collect::<String>()],
    )?;
    Ok(())
}

/// The visit ends: how long the tab stayed (spec 12 §2.1 step 5).
pub fn end_visit(conn: &Connection, visit_id: i64, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE visits SET duration_ms = MAX(0, ?2 - visited_at) WHERE id = ?1 AND duration_ms IS NULL",
        params![visit_id, now],
    )?;
    Ok(())
}

/// Stores the page's text for search (spec 12 §2.2), replacing an older one.
pub fn index_text(
    conn: &Connection,
    url: &str,
    title: &str,
    body: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    let Some(url_id): Option<i64> = conn
        .query_row("SELECT id FROM urls WHERE url = ?1", [key(url)], |r| {
            r.get(0)
        })
        .optional()?
    else {
        return Ok(false);
    };
    let body = truncate(body, 200 * 1024);
    conn.execute("DELETE FROM page_text WHERE rowid = ?1", [url_id])?;
    conn.execute(
        "INSERT INTO page_text (rowid, title, url, body) VALUES (?1, ?2, ?3, ?4)",
        params![url_id, title, key(url), body],
    )?;
    conn.execute(
        "INSERT INTO page_text_meta (url_id, bytes, indexed_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(url_id) DO UPDATE SET bytes = ?2, indexed_at = ?3",
        params![url_id, i64::try_from(body.len()).unwrap_or(i64::MAX), now],
    )?;
    Ok(true)
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// `history.query` (spec 12 §2.3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    pub text: Option<String>,
    pub host: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: u32,
    /// The `visitedAt` and visit ID of the last row of the previous page.
    pub cursor: Option<(i64, i64)>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Visit {
    pub visit_id: i64,
    pub url_id: i64,
    pub url: String,
    pub title: String,
    pub visited_at: i64,
    pub transition: Transition,
}

/// FTS5 query text from user words: each word quoted, so no word is read
/// as an operator.
fn fts_terms(text: &str) -> Option<String> {
    let mut out = String::new();
    for word in text.split_whitespace().take(16) {
        let word = word.replace('"', "\"\"");
        if !out.is_empty() {
            out.push(' ');
        }
        let _ = write!(out, "\"{word}\"");
    }
    (!out.is_empty()).then_some(out)
}

fn like(text: &str) -> String {
    let escaped: String = text
        .chars()
        .flat_map(|c| match c {
            '%' | '_' | '\\' => vec!['\\', c],
            c => vec![c],
        })
        .collect();
    format!("%{escaped}%")
}

/// Visits newest first. Text matches title, URL or the page's text; with
/// text, pages rank by bm25 plus frecency within the page of results.
pub fn query(conn: &Connection, q: &Query) -> rusqlite::Result<Vec<Visit>> {
    let limit = i64::from(q.limit.clamp(1, 500));
    let text = q.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let (before_at, before_id) = q.cursor.unwrap_or((i64::MAX, i64::MAX));
    let host = q
        .host
        .as_deref()
        .map(|h| h.trim_start_matches("www.").to_ascii_lowercase());
    let sql = "SELECT v.id, u.id, u.url, COALESCE(u.title, ''), v.visited_at, v.transition
         FROM visits v JOIN urls u ON u.id = v.url_id
         WHERE (v.visited_at, v.id) < (?1, ?2)
           AND (?3 IS NULL OR u.host = ?3 OR u.host LIKE '%.' || ?3)
           AND (?4 IS NULL OR v.visited_at >= ?4)
           AND (?5 IS NULL OR v.visited_at < ?5)
           AND (?6 IS NULL
                OR u.title LIKE ?7 ESCAPE '\\' OR u.url LIKE ?7 ESCAPE '\\'
                OR u.id IN (SELECT rowid FROM page_text WHERE page_text MATCH ?6))
           AND v.transition != 'redirect'
         ORDER BY v.visited_at DESC, v.id DESC
         LIMIT ?8";
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(
        params![
            before_at,
            before_id,
            host,
            q.from,
            q.to,
            text.and_then(fts_terms),
            text.map(like),
            limit
        ],
        |r| {
            Ok(Visit {
                visit_id: r.get(0)?,
                url_id: r.get(1)?,
                url: r.get(2)?,
                title: r.get(3)?,
                visited_at: r.get(4)?,
                transition: Transition::parse(&r.get::<_, String>(5)?),
            })
        },
    )?;
    rows.collect()
}

/// A suggestion row for the address bar (spec 05 §2).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub url_id: i64,
    pub url: String,
    pub title: String,
    pub frecency: f64,
    pub typed_count: i64,
}

/// History rows for typed input, best first (spec 05 §2): host prefix,
/// then title or URL containing every word. Within the 30 ms budget for a
/// 50,000-URL history (05 §12 check 2).
pub fn suggest(conn: &Connection, input: &str, limit: u32) -> rusqlite::Result<Vec<Suggestion>> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Vec::new());
    }
    let lower = input.to_ascii_lowercase();
    let prefix = lower
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    let mut stmt = conn.prepare_cached(
        "SELECT id, url, COALESCE(title, ''), frecency, typed_count FROM urls
         WHERE hidden = 0 AND host >= ?1 AND host < ?1 || x'ff'
         ORDER BY frecency DESC LIMIT ?2",
    )?;
    let map = |r: &rusqlite::Row<'_>| {
        Ok(Suggestion {
            url_id: r.get(0)?,
            url: r.get(1)?,
            title: r.get(2)?,
            frecency: r.get(3)?,
            typed_count: r.get(4)?,
        })
    };
    let limit = i64::from(limit.clamp(1, 50));
    let mut out: Vec<Suggestion> = stmt
        .query_map(params![prefix, limit], map)?
        .collect::<rusqlite::Result<_>>()?;
    if out.len() < usize::try_from(limit).unwrap_or(8) {
        let words: Vec<String> = lower.split_whitespace().take(6).map(like).collect();
        let mut sql = String::from(
            "SELECT id, url, COALESCE(title, ''), frecency, typed_count FROM urls WHERE hidden = 0 AND frecency > 0",
        );
        for i in 0..words.len() {
            let _ = write!(
                sql,
                " AND (title LIKE ?{n} ESCAPE '\\' OR url LIKE ?{n} ESCAPE '\\')",
                n = i + 1
            );
        }
        let _ = write!(sql, " ORDER BY frecency DESC LIMIT {}", limit * 2);
        let mut stmt = conn.prepare_cached(&sql)?;
        for s in stmt.query_map(rusqlite::params_from_iter(words.iter()), map)? {
            let s = s?;
            if !out.iter().any(|o| o.url_id == s.url_id)
                && out.len() < usize::try_from(limit).unwrap_or(8)
            {
                out.push(s);
            }
        }
    }
    Ok(out)
}

/// `history.delete` (spec 12 §2.3). Returns how many visits went.
pub fn delete(
    conn: &Connection,
    visit_ids: &[i64],
    url_ids: &[i64],
    host: Option<&str>,
    range: Option<(i64, i64)>,
) -> rusqlite::Result<usize> {
    let mut n = 0;
    for id in visit_ids {
        n += conn.execute("DELETE FROM visits WHERE id = ?1", [id])?;
    }
    for id in url_ids {
        n += conn.execute("DELETE FROM visits WHERE url_id = ?1", [id])?;
    }
    if let Some(host) = host {
        let host = host.trim_start_matches("www.").to_ascii_lowercase();
        n += conn.execute(
            "DELETE FROM visits WHERE url_id IN (SELECT id FROM urls WHERE host = ?1 OR host LIKE '%.' || ?1)",
            [&host],
        )?;
    }
    if let Some((from, to)) = range {
        n += conn.execute(
            "DELETE FROM visits WHERE visited_at >= ?1 AND visited_at < ?2",
            params![from, to],
        )?;
    }
    drop_orphans(conn)?;
    Ok(n)
}

/// URLs with no visits left go, with their text (spec 12 §2.4).
fn drop_orphans(conn: &Connection) -> rusqlite::Result<()> {
    let orphans: Vec<i64> = conn
        .prepare("SELECT id FROM urls WHERE id NOT IN (SELECT DISTINCT url_id FROM visits)")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in orphans {
        conn.execute("DELETE FROM page_text WHERE rowid = ?1", [id])?;
        conn.execute("DELETE FROM page_text_meta WHERE url_id = ?1", [id])?;
        conn.execute("DELETE FROM search_terms WHERE url_id = ?1", [id])?;
        conn.execute("DELETE FROM urls WHERE id = ?1", [id])?;
    }
    Ok(())
}

/// `history.exclude`: the host is never recorded again, and what is
/// recorded goes.
pub fn exclude(conn: &Connection, host: &str, now: i64) -> rusqlite::Result<()> {
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    conn.execute(
        "INSERT INTO history_exclusions (host, created_at) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        params![host, now],
    )?;
    delete(conn, &[], &[], Some(&host), None)?;
    Ok(())
}

/// `history.export`: CSV `visited_at_iso,title,url,transition`.
pub fn export_csv(conn: &Connection) -> rusqlite::Result<String> {
    let mut out = String::from("visited_at_iso,title,url,transition\n");
    let mut stmt = conn.prepare(
        "SELECT v.visited_at, COALESCE(u.title, ''), u.url, v.transition FROM visits v JOIN urls u ON u.id = v.url_id ORDER BY v.visited_at",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (at, title, url, t) = row?;
        let _ = writeln!(out, "{},{},{},{}", iso(at), csv(&title), csv(&url), t);
    }
    Ok(out)
}

fn csv(field: &str) -> String {
    // A leading = + - @ would run as a formula in a spreadsheet.
    let field = if field.starts_with(['=', '+', '-', '@']) {
        format!("'{field}")
    } else {
        field.to_owned()
    };
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field
    }
}

/// Unix milliseconds as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The daily job (spec 12 §2.4, 03 §7): old visits, orphans, then the
/// page text index back under 90% of its limit.
pub fn retention(
    conn: &Connection,
    keep_ms: i64,
    index_limit_bytes: i64,
    now: i64,
) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM visits WHERE visited_at < ?1", [now - keep_ms])?;
    drop_orphans(conn)?;
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(bytes), 0) FROM page_text_meta",
        [],
        |r| r.get(0),
    )?;
    if total > index_limit_bytes {
        let target = index_limit_bytes / 10 * 9;
        let mut over = total - target;
        let oldest: Vec<(i64, i64)> = conn
            .prepare("SELECT url_id, bytes FROM page_text_meta ORDER BY indexed_at")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, bytes) in oldest {
            if over <= 0 {
                break;
            }
            conn.execute("DELETE FROM page_text WHERE rowid = ?1", [id])?;
            conn.execute("DELETE FROM page_text_meta WHERE url_id = ?1", [id])?;
            over -= bytes;
        }
    }
    Ok(())
}

/// Hosts whose pages are never indexed (spec 12 §2.2):
/// `lists/sensitive-hosts.json`.
pub fn is_sensitive(host: &str) -> bool {
    let host = host.trim_start_matches("www.").to_ascii_lowercase();
    mado_lists::sensitive_hosts()
        .banking
        .iter()
        .any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("dir");
        let opened = mado_storage::open(
            &dir.path().join("p.sqlite"),
            mado_storage::Kind::Profile,
            &dir.path().join("b"),
            &dir.path().join("x"),
        )
        .expect("open");
        let conn = Connection::open(&opened.path).expect("conn");
        (dir, conn)
    }

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn visits_record_count_and_rank() {
        let (_d, c) = db();
        let a = record(
            &c,
            "https://a.example/x#frag",
            "A",
            Transition::Typed,
            "t",
            None,
            NOW,
        )
        .expect("ok")
        .expect("recorded");
        record(
            &c,
            "https://a.example/x",
            "",
            Transition::Link,
            "t",
            Some(a.visit_id),
            NOW,
        )
        .expect("ok");
        let (count, typed, title, frecency): (i64, i64, String, f64) = c
            .query_row(
                "SELECT visit_count, typed_count, title, frecency FROM urls WHERE id = ?1",
                [a.url_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .expect("row");
        assert_eq!((count, typed, title.as_str()), (2, 1, "A"));
        assert!((frecency - 300.0).abs() < 1e-9, "{frecency}");
    }

    #[test]
    fn excluded_hosts_are_not_recorded_and_lose_their_history() {
        let (_d, c) = db();
        record(
            &c,
            "https://www.secret.example/",
            "S",
            Transition::Link,
            "t",
            None,
            NOW,
        )
        .expect("ok");
        exclude(&c, "secret.example", NOW).expect("exclude");
        assert!(
            record(
                &c,
                "https://secret.example/2",
                "S",
                Transition::Link,
                "t",
                None,
                NOW
            )
            .expect("ok")
            .is_none()
        );
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM urls", [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 0);
    }

    #[test]
    fn page_text_finds_a_phrase_only_in_the_body() {
        let (_d, c) = db();
        record(
            &c,
            "https://b.example/",
            "Plain title",
            Transition::Link,
            "t",
            None,
            NOW,
        )
        .expect("ok");
        record(
            &c,
            "https://c.example/",
            "Other",
            Transition::Link,
            "t",
            None,
            NOW,
        )
        .expect("ok");
        assert!(
            index_text(
                &c,
                "https://b.example/",
                "Plain title",
                "the quick zebracorn jumps",
                NOW
            )
            .expect("index")
        );
        let q = |t: &str| {
            query(
                &c,
                &Query {
                    text: Some(t.into()),
                    limit: 10,
                    ..Query::default()
                },
            )
            .expect("query")
        };
        assert_eq!(
            q("zebracorn")
                .iter()
                .map(|v| v.url.as_str())
                .collect::<Vec<_>>(),
            ["https://b.example/"]
        );
        assert_eq!(q("other").len(), 1);
        // Operators are words, never syntax.
        assert!(q("zebracorn OR \"").is_empty());
    }

    #[test]
    fn delete_retention_and_export() {
        let (_d, c) = db();
        record(
            &c,
            "https://old.example/",
            "Old",
            Transition::Link,
            "t",
            None,
            NOW - 400 * DAY,
        )
        .expect("ok");
        record(
            &c,
            "https://new.example/",
            "=cmd()",
            Transition::Link,
            "t",
            None,
            NOW,
        )
        .expect("ok");
        retention(&c, 365 * DAY, 2 << 30, NOW).expect("retention");
        let csv = export_csv(&c).expect("csv");
        assert_eq!(
            csv,
            "visited_at_iso,title,url,transition\n2027-01-15T08:00:00Z,'=cmd(),https://new.example/,link\n"
        );
        assert_eq!(
            delete(&c, &[], &[], Some("new.example"), None).expect("delete"),
            1
        );
        assert!(export_csv(&c).expect("csv").lines().count() == 1);
    }

    #[test]
    fn banking_hosts_are_sensitive() {
        assert!(is_sensitive("www.chase.com"));
        assert!(is_sensitive("secure.chase.com"));
        assert!(!is_sensitive("example.com"));
    }

    /// 05 §12 check 2 and the M2-T10 budget: suggestions from a
    /// 50,000-URL history within 30 ms.
    #[test]
    fn suggestions_over_fifty_thousand_urls_stay_within_thirty_ms() {
        let (_d, c) = db();
        c.execute_batch("BEGIN").expect("begin");
        for i in 0..50_000 {
            c.execute(
                "INSERT INTO urls (url, host, title, visit_count, frecency) VALUES (?1, ?2, ?3, 1, ?4)",
                params![format!("https://site{i}.example/page"), format!("site{i}.example"), format!("Page number {i}"), f64::from(i % 997)],
            )
            .expect("insert");
        }
        c.execute_batch("COMMIT").expect("commit");
        suggest(&c, "site1", 8).expect("warm");
        for input in ["site12", "number 4", "zzz"] {
            // Best of three, so a busy test machine does not decide.
            let took = (0..3)
                .map(|_| {
                    let started = std::time::Instant::now();
                    let rows = suggest(&c, input, 8).expect("suggest");
                    assert!(rows.len() <= 8);
                    started.elapsed()
                })
                .min()
                .unwrap_or_default();
            assert!(took.as_millis() < 30, "{input}: {took:?}");
        }
    }
}
