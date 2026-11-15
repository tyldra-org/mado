//! Readers for other browsers' bookmarks, history, search engines, open
//! tabs and extensions (spec 12 §5). Pure: nothing here knows Mado's
//! state; the core applies what [`read`] returns.
//!
//! Roadmap: M2-T13.

mod arc;
mod chromium;
mod firefox;
mod safari;
mod snss;
mod sqlite;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use snss::parse as parse_snss;

/// Browsers Mado imports from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Browser {
    Chrome,
    Edge,
    Brave,
    Arc,
    Firefox,
    Safari,
}

impl Browser {
    pub const ALL: [Self; 6] = [
        Self::Chrome,
        Self::Edge,
        Self::Brave,
        Self::Arc,
        Self::Firefox,
        Self::Safari,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
            Self::Brave => "Brave",
            Self::Arc => "Arc",
            Self::Firefox => "Firefox",
            Self::Safari => "Safari",
        }
    }

    /// The Chromium user data folder under `~/Library/Application Support`.
    fn chromium_dir(self) -> Option<&'static str> {
        Some(match self {
            Self::Chrome => "Google/Chrome",
            Self::Edge => "Microsoft Edge",
            Self::Brave => "BraveSoftware/Brave-Browser",
            Self::Arc => "Arc/User Data",
            Self::Firefox | Self::Safari => return None,
        })
    }

    fn is_chromium(self) -> bool {
        self.chromium_dir().is_some()
    }
}

/// One profile of an installed browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub browser: Browser,
    /// The profile's folder name (`Default`, `Profile 1`, a Firefox
    /// profile path); empty for Safari.
    pub profile: String,
    /// What the browser calls the profile.
    pub name: String,
    #[serde(skip)]
    pub path: PathBuf,
}

/// What to import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Bookmarks,
    History,
    SearchEngines,
    Tabs,
    Extensions,
}

/// A bookmark, or a folder when `url` is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub title: String,
    pub url: Option<String>,
    pub children: Vec<Bookmark>,
}

impl Bookmark {
    pub fn link(title: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            url: Some(url.into()),
            children: Vec::new(),
        }
    }

    pub fn folder(title: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            title: title.into(),
            url: None,
            children,
        }
    }

    /// Links in this subtree.
    pub fn count(&self) -> usize {
        usize::from(self.url.is_some()) + self.children.iter().map(Self::count).sum::<usize>()
    }
}

/// One visit, at Unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    pub url: String,
    pub title: String,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine {
    pub name: String,
    pub keyword: String,
    /// With `{searchTerms}`.
    pub search_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    /// Tabs with the same group are grouped; the browser's own group key.
    pub group: Option<String>,
}

/// A saved set of tabs: an Arc space's pinned tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    pub title: String,
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    pub id: String,
    pub name: String,
}

/// Everything one run read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imported {
    pub bar: Vec<Bookmark>,
    pub other: Vec<Bookmark>,
    pub visits: Vec<Visit>,
    pub engines: Vec<Engine>,
    /// The default engine's name.
    pub default_engine: Option<String>,
    /// Open windows, each with its tabs in order.
    pub windows: Vec<Vec<Tab>>,
    pub spaces: Vec<Space>,
    pub extensions: Vec<Extension>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// macOS refused: Safari's data needs Full Disk Access.
    #[error("Full Disk Access is needed")]
    FullDiskAccess,
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("{0}: {1}")]
    Format(PathBuf, String),
    #[error("{0}: {1}")]
    Sqlite(PathBuf, rusqlite::Error),
}

impl ImportError {
    fn io(path: &Path, e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            Self::FullDiskAccess
        } else {
            Self::Io(path.to_owned(), e)
        }
    }

    fn format(path: &Path, what: impl Into<String>) -> Self {
        Self::Format(path.to_owned(), what.into())
    }
}

/// Microseconds since 1601-01-01 (Chromium) to Unix milliseconds.
pub fn from_chromium(us: i64) -> i64 {
    us / 1000 - 11_644_473_600_000
}

/// Microseconds since 1970 (Firefox) to Unix milliseconds.
pub fn from_firefox(us: i64) -> i64 {
    us / 1000
}

/// Seconds since 2001-01-01 (Safari, Core Data) to Unix milliseconds.
#[allow(clippy::cast_possible_truncation)]
pub fn from_safari(s: f64) -> i64 {
    ((s + 978_307_200.0) * 1000.0).round() as i64
}

/// Installed browsers and their profiles under `home` (spec 12 §5.1).
pub fn detect(home: &Path) -> Vec<Source> {
    let support = home.join("Library/Application Support");
    let mut out = Vec::new();
    for browser in Browser::ALL {
        match browser {
            b if b.is_chromium() => {
                let dir = support.join(b.chromium_dir().unwrap_or_default());
                out.extend(chromium::profiles(b, &dir));
            }
            Browser::Firefox => out.extend(firefox::profiles(&support.join("Firefox"))),
            _ => {
                let dir = home.join("Library/Safari");
                if dir.exists() {
                    out.push(Source {
                        browser: Browser::Safari,
                        profile: String::new(),
                        name: "Safari".into(),
                        path: dir,
                    });
                }
            }
        }
    }
    out
}

/// Reads `kinds` from `source`; history from `since` (Unix milliseconds)
/// on, or all of it. `home` locates files outside the profile folder
/// (Arc's sidebar, Safari's preferences).
pub fn read(
    source: &Source,
    kinds: &[Kind],
    since: Option<i64>,
    home: &Path,
) -> Result<Imported, ImportError> {
    let mut out = match source.browser {
        b if b.is_chromium() => chromium::read(&source.path, kinds, since)?,
        Browser::Firefox => firefox::read(&source.path, kinds, since)?,
        _ => safari::read(&source.path, kinds, since, home)?,
    };
    if source.browser == Browser::Arc && kinds.contains(&Kind::Tabs) {
        let sidebar = home.join("Library/Application Support/Arc/StorableSidebar.json");
        if sidebar.exists() {
            let (favorites, spaces) = arc::read(&sidebar)?;
            if !favorites.is_empty() {
                out.windows.insert(0, favorites);
            }
            out.spaces = spaces;
        }
    }
    Ok(out)
}

#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;
#[cfg(test)]
mod tests;
