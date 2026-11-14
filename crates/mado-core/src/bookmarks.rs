//! Bookmarks (spec 12 §3): two roots, every command's database work, and
//! the Netscape bookmark file format for export and import. Plain functions
//! over a SQLite connection; the core runs writes on the writer thread and
//! reads on the read pool.

use std::fmt::Write as _;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Folder,
    Link,
    Separator,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::Link => "link",
            Self::Separator => "separator",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "folder" => Self::Folder,
            "separator" => Self::Separator,
            _ => Self::Link,
        }
    }
}

/// One bookmark, folder or separator, with its children when asked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: Kind,
    pub title: String,
    pub url: Option<String>,
    pub index: u32,
    pub root: Option<String>,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub children: Option<Vec<Node>>,
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn invalid(what: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(what.to_owned())
}

/// The two roots (spec 12 §3.1), created on first use. Returns
/// `(bar, other)`.
pub fn roots(conn: &Connection, now: i64) -> rusqlite::Result<(String, String)> {
    let get = |root: &str, title: &str, sort: i64| -> rusqlite::Result<String> {
        if let Some(id) = conn
            .query_row("SELECT id FROM bookmarks WHERE root = ?1", [root], |r| {
                r.get(0)
            })
            .optional()?
        {
            return Ok(id);
        }
        let id = new_id();
        conn.execute(
            "INSERT INTO bookmarks (id, parent_id, kind, title, sort, root, created_at, updated_at)
             VALUES (?1, NULL, 'folder', ?2, ?3, ?4, ?5, ?5)",
            params![id, title, sort, root, now],
        )?;
        Ok(id)
    };
    Ok((
        get("bar", "Bookmarks bar", 0)?,
        get("other", "Other bookmarks", 1)?,
    ))
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    Ok(Node {
        id: r.get(0)?,
        parent_id: r.get(1)?,
        kind: Kind::parse(&r.get::<_, String>(2)?),
        title: r.get(3)?,
        url: r.get(4)?,
        index: u32::try_from(r.get::<_, i64>(5)?).unwrap_or(0),
        root: r.get(6)?,
        created_at: r.get(7)?,
        children: None,
    })
}

const COLUMNS: &str = "id, parent_id, kind, title, url, sort, root, created_at";

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<Node>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM bookmarks WHERE id = ?1"),
        [id],
        row,
    )
    .optional()
}

fn children(conn: &Connection, parent: &str) -> rusqlite::Result<Vec<Node>> {
    conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM bookmarks WHERE parent_id = ?1 ORDER BY sort"
    ))?
    .query_map([parent], row)?
    .collect()
}

fn subtree(conn: &Connection, mut node: Node) -> rusqlite::Result<Node> {
    if node.kind == Kind::Folder {
        let kids = children(conn, &node.id)?
            .into_iter()
            .map(|k| subtree(conn, k))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        node.children = Some(kids);
    }
    Ok(node)
}

/// `bookmark.tree`: the folder's subtree, or both roots.
pub fn tree(conn: &Connection, parent: Option<&str>, now: i64) -> rusqlite::Result<Vec<Node>> {
    match parent {
        Some(p) => {
            let node = get(conn, p)?.ok_or_else(|| invalid("parentId"))?;
            Ok(vec![subtree(conn, node)?])
        }
        None => {
            let (bar, other) = roots(conn, now)?;
            [bar, other]
                .iter()
                .map(|id| {
                    let n = get(conn, id)?.ok_or_else(|| invalid("root"))?;
                    subtree(conn, n)
                })
                .collect()
        }
    }
}

/// Renumbers a folder's children 0, 1, 2 in their current order, with
/// `insert` placed at `index`.
fn place(conn: &Connection, parent: &str, insert: Option<(&str, usize)>) -> rusqlite::Result<()> {
    let mut ids: Vec<String> = conn
        .prepare("SELECT id FROM bookmarks WHERE parent_id = ?1 ORDER BY sort")?
        .query_map([parent], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if let Some((id, index)) = insert {
        ids.retain(|x| x != id);
        let at = index.min(ids.len());
        ids.insert(at, id.to_owned());
    }
    for (i, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE bookmarks SET sort = ?2 WHERE id = ?1",
            params![id, i64::try_from(i).unwrap_or(0)],
        )?;
    }
    Ok(())
}

fn folder_exists(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    Ok(get(conn, id)?.is_some_and(|n| n.kind == Kind::Folder))
}

/// The folder a new bookmark goes in by default: the last one used, else
/// Other bookmarks (spec 12 §3.2).
pub fn default_parent(conn: &Connection, now: i64) -> rusqlite::Result<String> {
    let last: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM bookmarks WHERE kind = 'link' AND parent_id IS NOT NULL ORDER BY created_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    match last {
        Some(p) if folder_exists(conn, &p)? => Ok(p),
        _ => Ok(roots(conn, now)?.1),
    }
}

/// Adds a link, folder or separator. Returns its ID.
pub fn add(
    conn: &Connection,
    parent: Option<&str>,
    kind: Kind,
    title: &str,
    url: Option<&str>,
    index: Option<usize>,
    now: i64,
) -> rusqlite::Result<String> {
    let parent = match parent {
        Some(p) if folder_exists(conn, p)? => p.to_owned(),
        Some(_) => return Err(invalid("parentId")),
        None => default_parent(conn, now)?,
    };
    if kind == Kind::Link && url.is_none_or(|u| u.trim().is_empty()) {
        return Err(invalid("url"));
    }
    let id = new_id();
    let end: i64 = conn.query_row(
        "SELECT COUNT(*) FROM bookmarks WHERE parent_id = ?1",
        [&parent],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO bookmarks (id, parent_id, kind, title, url, sort, root, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?7)",
        params![id, parent, kind.as_str(), title.chars().take(1024).collect::<String>(), url, end, now],
    )?;
    if let Some(i) = index {
        place(conn, &parent, Some((&id, i)))?;
    }
    Ok(id)
}

pub fn update(
    conn: &Connection,
    id: &str,
    title: Option<&str>,
    url: Option<&str>,
    now: i64,
) -> rusqlite::Result<()> {
    let node = get(conn, id)?.ok_or_else(|| invalid("id"))?;
    if node.root.is_some() {
        return Err(invalid("root"));
    }
    if let Some(t) = title {
        conn.execute(
            "UPDATE bookmarks SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, t.chars().take(1024).collect::<String>(), now],
        )?;
    }
    if let Some(u) = url
        && node.kind == Kind::Link
    {
        conn.execute(
            "UPDATE bookmarks SET url = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, u, now],
        )?;
    }
    Ok(())
}

/// `bookmark.move`. A folder cannot move into itself or its descendants.
pub fn move_to(
    conn: &Connection,
    id: &str,
    parent: &str,
    index: usize,
    now: i64,
) -> rusqlite::Result<()> {
    let node = get(conn, id)?.ok_or_else(|| invalid("id"))?;
    if node.root.is_some() || !folder_exists(conn, parent)? {
        return Err(invalid("parentId"));
    }
    let mut at = Some(parent.to_owned());
    while let Some(p) = at {
        if p == id {
            return Err(invalid("parentId"));
        }
        at = get(conn, &p)?.and_then(|n| n.parent_id);
    }
    let old = node.parent_id.unwrap_or_default();
    conn.execute(
        "UPDATE bookmarks SET parent_id = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, parent, now],
    )?;
    place(conn, parent, Some((id, index)))?;
    if old != parent {
        place(conn, &old, None)?;
    }
    Ok(())
}

/// Deletes nodes with their subtrees. Returns what went, for Undo.
pub fn delete(conn: &Connection, ids: &[String]) -> rusqlite::Result<Vec<Node>> {
    let mut gone = Vec::new();
    for id in ids {
        let Some(node) = get(conn, id)? else { continue };
        if node.root.is_some() {
            return Err(invalid("root"));
        }
        let parent = node.parent_id.clone().unwrap_or_default();
        gone.push(subtree(conn, node)?);
        conn.execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        place(conn, &parent, None)?;
    }
    Ok(gone)
}

/// Puts deleted nodes back where they were (Undo).
pub fn restore(conn: &Connection, nodes: &[Node], now: i64) -> rusqlite::Result<()> {
    fn insert(conn: &Connection, n: &Node, now: i64) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO bookmarks (id, parent_id, kind, title, url, sort, root, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8) ON CONFLICT(id) DO NOTHING",
            params![n.id, n.parent_id, n.kind.as_str(), n.title, n.url, n.index, n.created_at, now],
        )?;
        for c in n.children.iter().flatten() {
            insert(conn, c, now)?;
        }
        Ok(())
    }
    for n in nodes {
        let parent = n.parent_id.clone().unwrap_or_default();
        if !folder_exists(conn, &parent)? {
            continue;
        }
        insert(conn, n, now)?;
        place(conn, &parent, Some((&n.id, n.index as usize)))?;
    }
    Ok(())
}

/// `bookmark.sort`: folders first, then by name or URL.
pub fn sort(conn: &Connection, folder: &str, by_url: bool) -> rusqlite::Result<()> {
    let mut kids = children(conn, folder)?;
    kids.sort_by(|a, b| {
        (a.kind != Kind::Folder)
            .cmp(&(b.kind != Kind::Folder))
            .then_with(|| {
                let key = |n: &Node| {
                    if by_url {
                        n.url.clone().unwrap_or_default()
                    } else {
                        n.title.to_lowercase()
                    }
                };
                key(a).cmp(&key(b))
            })
    });
    for (i, k) in kids.iter().enumerate() {
        conn.execute(
            "UPDATE bookmarks SET sort = ?2 WHERE id = ?1",
            params![k.id, i64::try_from(i).unwrap_or(0)],
        )?;
    }
    Ok(())
}

pub fn search(conn: &Connection, query: &str, limit: u32) -> rusqlite::Result<Vec<Node>> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let pattern = format!(
        "%{}%",
        q.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM bookmarks WHERE kind = 'link' AND (title LIKE ?1 ESCAPE '\\' OR url LIKE ?1 ESCAPE '\\')
         ORDER BY created_at DESC LIMIT ?2"
    ))?
    .query_map(params![pattern, i64::from(limit.clamp(1, 500))], row)?
    .collect()
}

pub fn recent(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<Node>> {
    conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM bookmarks WHERE kind = 'link' ORDER BY created_at DESC LIMIT ?1"
    ))?
    .query_map([i64::from(limit.clamp(1, 500))], row)?
    .collect()
}

/// The bookmarks of a URL, for ⌘D's "already bookmarked".
pub fn of_url(conn: &Connection, url: &str) -> rusqlite::Result<Vec<Node>> {
    conn.prepare_cached(&format!("SELECT {COLUMNS} FROM bookmarks WHERE url = ?1"))?
        .query_map([url], row)?
        .collect()
}

/// The links in a folder, in order (`bookmark.openFolder`).
pub fn links_in(conn: &Connection, folder: &str) -> rusqlite::Result<Vec<String>> {
    Ok(children(conn, folder)?
        .into_iter()
        .filter(|n| n.kind == Kind::Link)
        .filter_map(|n| n.url)
        .collect())
}

// ----- the Netscape bookmark file format (spec 12 §3.3) -----

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|e| *e <= 10);
        let entity = end.map(|e| &rest[1..e]);
        let decoded = match entity {
            Some("amp") => Some('&'),
            Some("lt") => Some('<'),
            Some("gt") => Some('>'),
            Some("quot") => Some('"'),
            Some("apos" | "#39") => Some('\''),
            Some(e) if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            Some(e) if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match (decoded, end) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn secs(ms: i64) -> i64 {
    ms / 1000
}

/// `bookmark.export`.
pub fn export_html(conn: &Connection, now: i64) -> rusqlite::Result<String> {
    fn write_items(out: &mut String, nodes: &[Node], depth: usize) {
        let pad = "    ".repeat(depth);
        for n in nodes {
            match n.kind {
                Kind::Folder => {
                    let _ = writeln!(
                        out,
                        "{pad}<DT><H3 ADD_DATE=\"{}\">{}</H3>",
                        secs(n.created_at),
                        escape(&n.title)
                    );
                    let _ = writeln!(out, "{pad}<DL><p>");
                    write_items(out, n.children.as_deref().unwrap_or_default(), depth + 1);
                    let _ = writeln!(out, "{pad}</DL><p>");
                }
                Kind::Link => {
                    let _ = writeln!(
                        out,
                        "{pad}<DT><A HREF=\"{}\" ADD_DATE=\"{}\">{}</A>",
                        escape(n.url.as_deref().unwrap_or_default()),
                        secs(n.created_at),
                        escape(&n.title)
                    );
                }
                Kind::Separator => {
                    let _ = writeln!(out, "{pad}<HR>");
                }
            }
        }
    }
    let roots = tree(conn, None, now)?;
    let mut out = String::from(
        "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n<TITLE>Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n<DL><p>\n",
    );
    for r in &roots {
        let bar = r.root.as_deref() == Some("bar");
        if bar {
            let _ = writeln!(
                out,
                "    <DT><H3 ADD_DATE=\"{}\" PERSONAL_TOOLBAR_FOLDER=\"true\">{}</H3>\n    <DL><p>",
                secs(r.created_at),
                escape(&r.title)
            );
            write_items(&mut out, r.children.as_deref().unwrap_or_default(), 2);
            out.push_str("    </DL><p>\n");
        } else {
            write_items(&mut out, r.children.as_deref().unwrap_or_default(), 1);
        }
    }
    out.push_str("</DL><p>\n");
    Ok(out)
}

/// A parsed bookmark file node.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Folder {
        title: String,
        toolbar: bool,
        added: Option<i64>,
        items: Vec<Item>,
    },
    Link {
        title: String,
        url: String,
        added: Option<i64>,
    },
    Separator,
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let upper = tag.to_ascii_uppercase();
    let at = upper.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[at..];
    let value = if let Some(r) = rest.strip_prefix('"') {
        &r[..r.find('"')?]
    } else {
        rest.split([' ', '>']).next()?
    };
    Some(unescape(value))
}

/// Parses a Netscape bookmark file into its top-level items. Tolerant:
/// unknown tags are skipped, unclosed lists end at the end of the file.
pub fn parse_html(html: &str) -> Vec<Item> {
    let mut stack: Vec<Vec<Item>> = vec![Vec::new()];
    let mut pending: Option<(String, bool, Option<i64>)> = None;
    let mut open_folders: Vec<(String, bool, Option<i64>)> = Vec::new();
    let mut started = false;
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        rest = &rest[i..];
        let end = rest.find('>').unwrap_or(rest.len() - 1);
        let tag = &rest[1..end];
        let name = tag
            .split([' ', '\t', '\n'])
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let after = &rest[(end + 1).min(rest.len())..];
        match name.as_str() {
            "H3" => {
                let close = after
                    .to_ascii_uppercase()
                    .find("</H3>")
                    .unwrap_or(after.len());
                let title = unescape(after[..close].trim());
                let toolbar = attr(tag, "PERSONAL_TOOLBAR_FOLDER")
                    .is_some_and(|v| v.eq_ignore_ascii_case("true"));
                let added = attr(tag, "ADD_DATE")
                    .and_then(|d| d.parse::<i64>().ok())
                    .map(|s| s * 1000);
                pending = Some((title, toolbar, added));
            }
            "A" => {
                let close = after
                    .to_ascii_uppercase()
                    .find("</A>")
                    .unwrap_or(after.len());
                let title = unescape(after[..close].trim());
                if let (Some(url), Some(top)) = (attr(tag, "HREF"), stack.last_mut()) {
                    let added = attr(tag, "ADD_DATE")
                        .and_then(|d| d.parse::<i64>().ok())
                        .map(|s| s * 1000);
                    top.push(Item::Link { title, url, added });
                }
            }
            "HR" => {
                if let Some(top) = stack.last_mut() {
                    top.push(Item::Separator);
                }
            }
            "DL" => {
                if started {
                    let folder = pending
                        .take()
                        .unwrap_or_else(|| (String::new(), false, None));
                    open_folders.push(folder);
                    stack.push(Vec::new());
                } else {
                    started = true;
                }
            }
            "/DL" => {
                // The outermost list's close has no folder of its own.
                if let Some((title, toolbar, added)) = open_folders.pop()
                    && let Some(items) = stack.pop()
                    && let Some(top) = stack.last_mut()
                {
                    top.push(Item::Folder {
                        title,
                        toolbar,
                        added,
                        items,
                    });
                }
            }
            _ => {}
        }
        rest = after;
    }
    while stack.len() > 1 {
        let items = stack.pop().unwrap_or_default();
        let (title, toolbar, added) = open_folders.pop().unwrap_or_default();
        if let Some(top) = stack.last_mut() {
            top.push(Item::Folder {
                title,
                toolbar,
                added,
                items,
            });
        }
    }
    stack.pop().unwrap_or_default()
}

/// Whether only the two empty roots exist.
pub fn is_empty(conn: &Connection) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM bookmarks WHERE root IS NULL",
        [],
        |r| r.get(0),
    )?;
    Ok(n == 0)
}

/// `bookmark.import` (spec 12 §3.3): into the roots when the profile has
/// no bookmarks, else under a new "Imported" folder in Other bookmarks.
/// Returns how many links were added.
pub fn import(conn: &Connection, items: &[Item], now: i64) -> rusqlite::Result<usize> {
    fn put(
        conn: &Connection,
        parent: &str,
        items: &[Item],
        now: i64,
        count: &mut usize,
    ) -> rusqlite::Result<()> {
        for item in items {
            match item {
                Item::Folder {
                    title,
                    items,
                    added,
                    ..
                } => {
                    let id = add(
                        conn,
                        Some(parent),
                        Kind::Folder,
                        title,
                        None,
                        None,
                        added.unwrap_or(now),
                    )?;
                    put(conn, &id, items, now, count)?;
                }
                Item::Link { title, url, added } => {
                    if url::Url::parse(url).is_ok() {
                        add(
                            conn,
                            Some(parent),
                            Kind::Link,
                            title,
                            Some(url),
                            None,
                            added.unwrap_or(now),
                        )?;
                        *count += 1;
                    }
                }
                Item::Separator => {
                    add(conn, Some(parent), Kind::Separator, "", None, None, now)?;
                }
            }
        }
        Ok(())
    }
    let (bar, other) = roots(conn, now)?;
    let mut count = 0;
    if is_empty(conn)? {
        for item in items {
            match item {
                Item::Folder {
                    toolbar: true,
                    items,
                    ..
                } => put(conn, &bar, items, now, &mut count)?,
                other_item => put(
                    conn,
                    &other,
                    std::slice::from_ref(other_item),
                    now,
                    &mut count,
                )?,
            }
        }
    } else {
        let folder = add(
            conn,
            Some(&other),
            Kind::Folder,
            "Imported",
            None,
            None,
            now,
        )?;
        put(conn, &folder, items, now, &mut count)?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

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

    /// Titles and URLs of a tree, without IDs and times.
    fn shape(nodes: &[Node]) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(nodes: &[Node], depth: usize, out: &mut Vec<String>) {
            for n in nodes {
                out.push(format!(
                    "{}{:?} {} {}",
                    "  ".repeat(depth),
                    n.kind,
                    n.title,
                    n.url.clone().unwrap_or_default()
                ));
                walk(n.children.as_deref().unwrap_or_default(), depth + 1, out);
            }
        }
        walk(nodes, 0, &mut out);
        out
    }

    fn sample(c: &Connection) {
        let (bar, other) = roots(c, NOW).expect("roots");
        add(
            c,
            Some(&bar),
            Kind::Link,
            "News & \"stuff\" <b>",
            Some("https://news.example/?a=1&b=2"),
            None,
            NOW,
        )
        .expect("add");
        let f = add(c, Some(&bar), Kind::Folder, "Work", None, None, NOW).expect("folder");
        add(
            c,
            Some(&f),
            Kind::Link,
            "Docs",
            Some("https://docs.example/"),
            None,
            NOW,
        )
        .expect("add");
        add(c, Some(&f), Kind::Separator, "", None, None, NOW).expect("sep");
        let deep = add(c, Some(&f), Kind::Folder, "Deep", None, None, NOW).expect("folder");
        add(
            c,
            Some(&deep),
            Kind::Link,
            "Ünïcode ✓",
            Some("https://ü.example/"),
            None,
            NOW,
        )
        .expect("add");
        add(
            c,
            Some(&other),
            Kind::Link,
            "Other one",
            Some("https://other.example/"),
            None,
            NOW,
        )
        .expect("add");
    }

    /// 12 §7 check 4: export, then import into an empty profile, gives an
    /// identical tree.
    #[test]
    fn export_then_import_into_an_empty_profile_is_identical() {
        let (_a, a) = db();
        sample(&a);
        let html = export_html(&a, NOW).expect("export");
        let (_b, b) = db();
        let n = import(&b, &parse_html(&html), NOW).expect("import");
        assert_eq!(n, 4);
        assert_eq!(
            shape(&tree(&b, None, NOW).expect("tree")),
            shape(&tree(&a, None, NOW).expect("tree"))
        );
    }

    #[test]
    fn import_into_a_profile_with_bookmarks_goes_under_imported() {
        let (_a, a) = db();
        sample(&a);
        let html = export_html(&a, NOW).expect("export");
        import(&a, &parse_html(&html), NOW).expect("import");
        let other = &tree(&a, None, NOW).expect("tree")[1];
        let kids = other.children.as_deref().unwrap_or_default();
        assert_eq!(kids.last().map(|k| k.title.as_str()), Some("Imported"));
    }

    #[test]
    fn move_sort_delete_and_undo() {
        let (_d, c) = db();
        let (bar, _) = roots(&c, NOW).expect("roots");
        let b = add(
            &c,
            Some(&bar),
            Kind::Link,
            "B",
            Some("https://b/"),
            None,
            NOW,
        )
        .expect("b");
        let f = add(&c, Some(&bar), Kind::Folder, "F", None, None, NOW).expect("f");
        let a = add(
            &c,
            Some(&bar),
            Kind::Link,
            "a",
            Some("https://a/"),
            Some(0),
            NOW,
        )
        .expect("a");
        let titles = |c: &Connection| {
            children(c, &bar)
                .expect("kids")
                .into_iter()
                .map(|n| n.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(titles(&c), ["a", "B", "F"]);
        sort(&c, &bar, false).expect("sort");
        assert_eq!(titles(&c), ["F", "a", "B"]);
        assert!(
            move_to(&c, &f, &f, 0, NOW).is_err(),
            "a folder cannot hold itself"
        );
        move_to(&c, &a, &f, 0, NOW).expect("move");
        assert_eq!(titles(&c), ["F", "B"]);
        let gone = delete(&c, std::slice::from_ref(&f)).expect("delete");
        assert_eq!(titles(&c), ["B"]);
        restore(&c, &gone, NOW).expect("undo");
        assert_eq!(titles(&c), ["F", "B"]);
        assert_eq!(links_in(&c, &f).expect("links"), ["https://a/"]);
        assert!(delete(&c, &[bar]).is_err(), "roots stay");
        let _ = b;
    }

    #[test]
    fn a_chrome_export_parses() {
        let html = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<DL><p>
    <DT><H3 ADD_DATE="1" PERSONAL_TOOLBAR_FOLDER="true">Bookmarks bar</H3>
    <DL><p>
        <DT><A HREF="https://a.example/" ADD_DATE="2" ICON="data:x">A &amp; B</A>
        <DT><H3>Folder</H3>
        <DL><p>
            <DT><A HREF="https://c.example/">C</A>
        </DL><p>
    </DL><p>
    <DT><A HREF="https://d.example/">D</A>
</DL><p>"#;
        let items = parse_html(html);
        assert_eq!(items.len(), 2);
        let Item::Folder {
            toolbar,
            items: bar,
            ..
        } = &items[0]
        else {
            panic!("bar folder")
        };
        assert!(toolbar);
        assert_eq!(
            bar[0],
            Item::Link {
                title: "A & B".into(),
                url: "https://a.example/".into(),
                added: Some(2000)
            }
        );
    }
}
