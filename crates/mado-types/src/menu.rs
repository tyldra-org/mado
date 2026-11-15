//! The menu bar (spec 01 §5.4, 05 §10): Chrome's structure on macOS. Each
//! item is a command ID; the core runs it (`mado-core` menus) and the app
//! builds native menus from this table. Default key equivalents are the
//! `shortcuts.<id>` settings (mado-settings), so users can change them.

/// One entry of a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEntry {
    /// A Mado command: its ID, `en.json` key `menu.<id>`.
    Item(&'static str),
    /// A toggle with a check mark.
    Check(&'static str),
    /// One of the system's own items, which AppKit runs.
    System(SystemItem),
    Separator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemItem {
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Fullscreen,
    Minimize,
    Zoom,
    BringAllToFront,
}

/// A top-level menu: its `en.json` key `menu.title.<id>` and entries.
#[derive(Debug, Clone, Copy)]
pub struct MenuDef {
    pub id: &'static str,
    pub entries: &'static [MenuEntry],
}

use MenuEntry::{Check, Item, Separator, System};
use SystemItem as S;

/// The menu bar, in order. The first menu is the application menu.
pub const MENU_BAR: &[MenuDef] = &[
    MenuDef {
        id: "app",
        entries: &[
            Item("about"),
            Separator,
            Item("settings"),
            Separator,
            System(S::Services),
            Separator,
            System(S::Hide),
            System(S::HideOthers),
            System(S::ShowAll),
            Separator,
            Item("quit"),
        ],
    },
    MenuDef {
        id: "file",
        entries: &[
            Item("newTab"),
            Item("newWindow"),
            Item("newPrivateWindow"),
            Item("reopenClosedTab"),
            Item("openLocation"),
            Item("focusAddressBar"),
            Separator,
            Item("closeWindow"),
            Item("closeTab"),
            Separator,
            Item("importBrowserData"),
            Item("print"),
        ],
    },
    MenuDef {
        id: "edit",
        entries: &[
            System(S::Undo),
            System(S::Redo),
            Separator,
            System(S::Cut),
            System(S::Copy),
            System(S::Paste),
            System(S::SelectAll),
            Separator,
            Item("find"),
            Item("findNext"),
            Item("findPrevious"),
        ],
    },
    MenuDef {
        id: "view",
        entries: &[
            Check("toggleBookmarksBar"),
            Separator,
            Item("stop"),
            Item("reload"),
            Item("forceReload"),
            Separator,
            System(S::Fullscreen),
            Item("actualSize"),
            Item("zoomIn"),
            Item("zoomOut"),
            Separator,
            Item("reader"),
            Item("viewSource"),
            Item("developerTools"),
        ],
    },
    MenuDef {
        id: "history",
        entries: &[
            Item("home"),
            Item("back"),
            Item("forward"),
            Separator,
            Item("showHistory"),
        ],
    },
    MenuDef {
        id: "bookmarks",
        entries: &[
            Item("bookmarkManager"),
            Item("bookmarkTab"),
            Item("bookmarkAllTabs"),
        ],
    },
    MenuDef {
        id: "tab",
        entries: &[
            Item("nextTab"),
            Item("previousTab"),
            Separator,
            Item("selectTab1"),
            Item("selectTab2"),
            Item("selectTab3"),
            Item("selectTab4"),
            Item("selectTab5"),
            Item("selectTab6"),
            Item("selectTab7"),
            Item("selectTab8"),
            Item("selectLastTab"),
            Separator,
            Item("duplicateTab"),
            Check("pinTab"),
            Check("muteSite"),
            Item("moveTabToNewWindow"),
        ],
    },
    MenuDef {
        id: "window",
        entries: &[
            System(S::Minimize),
            System(S::Zoom),
            Separator,
            Item("downloads"),
            Separator,
            System(S::BringAllToFront),
        ],
    },
];

/// The Dock menu's items (macOS), besides the windows AppKit lists.
pub const DOCK_MENU: &[&str] = &["newWindow", "newPrivateWindow"];

/// Shortcuts pages never receive (spec 05 §10): the menu acts first.
pub const RESERVED: &[&str] = &[
    "newTab",
    "reopenClosedTab",
    "closeTab",
    "closeWindow",
    "newWindow",
    "newPrivateWindow",
    "quit",
    "openLocation",
    "nextTab",
    "previousTab",
    "selectTab1",
    "selectTab2",
    "selectTab3",
    "selectTab4",
    "selectTab5",
    "selectTab6",
    "selectTab7",
    "selectTab8",
    "selectLastTab",
];

/// Every Mado command ID in the menus, in order, once each.
pub fn command_ids() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for m in MENU_BAR {
        for e in m.entries {
            if let Item(id) | Check(id) = e
                && !out.contains(id)
            {
                out.push(id);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_reserved_ones_are_in_the_menus() {
        let ids = command_ids();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
        for r in RESERVED.iter().chain(DOCK_MENU) {
            assert!(ids.contains(r), "{r}");
        }
    }
}
