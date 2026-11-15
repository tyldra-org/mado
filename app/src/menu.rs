//! The native menu bar from `mado_types::menu` (spec 01 §5.4): labels
//! from `en.json`, key equivalents from the `shortcuts.*` settings, and
//! enabled and checked states the core recomputes after every change.

use std::collections::BTreeMap;
use std::sync::Mutex;

use mado_types::menu::{MENU_BAR, MenuEntry, SystemItem};
use tauri::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu,
};
use tauri::{AppHandle, Manager, Wry};

/// The items the core updates, by command ID.
#[derive(Default)]
pub struct MenuItems(pub Mutex<BTreeMap<String, MenuItemKind<Wry>>>);

fn system(app: &AppHandle, item: SystemItem) -> tauri::Result<PredefinedMenuItem<Wry>> {
    match item {
        SystemItem::Services => PredefinedMenuItem::services(app, None),
        SystemItem::Hide => PredefinedMenuItem::hide(app, None),
        SystemItem::HideOthers => PredefinedMenuItem::hide_others(app, None),
        SystemItem::ShowAll => PredefinedMenuItem::show_all(app, None),
        SystemItem::Undo => PredefinedMenuItem::undo(app, None),
        SystemItem::Redo => PredefinedMenuItem::redo(app, None),
        SystemItem::Cut => PredefinedMenuItem::cut(app, None),
        SystemItem::Copy => PredefinedMenuItem::copy(app, None),
        SystemItem::Paste => PredefinedMenuItem::paste(app, None),
        SystemItem::SelectAll => PredefinedMenuItem::select_all(app, None),
        SystemItem::Fullscreen => PredefinedMenuItem::fullscreen(app, None),
        SystemItem::Minimize => PredefinedMenuItem::minimize(app, None),
        SystemItem::Zoom => PredefinedMenuItem::maximize(app, None),
        SystemItem::BringAllToFront => PredefinedMenuItem::bring_all_to_front(app, None),
    }
}

/// A shortcut setting as Tauri's accelerator: `""` is none.
fn accelerator(keys: Option<&String>) -> Option<&str> {
    keys.map(String::as_str).filter(|k| !k.is_empty())
}

/// Builds the menu bar with the default shortcuts; the core sends the
/// user's once it starts.
pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let strings = mado_settings::Strings::english();
    let defaults: BTreeMap<String, String> = mado_settings::REGISTRY
        .iter()
        .filter_map(|d| {
            let id = d.key.strip_prefix("shortcuts.")?;
            Some((id.to_owned(), d.default_value().as_str()?.to_owned()))
        })
        .collect();
    let mut items = BTreeMap::new();
    let mut menus: Vec<Submenu<Wry>> = Vec::new();
    for def in MENU_BAR {
        let title = if def.id == "app" {
            "Mado".to_owned()
        } else {
            strings.get(&format!("menu.title.{}", def.id)).to_owned()
        };
        let submenu = Submenu::new(app, title, true)?;
        for entry in def.entries {
            match *entry {
                MenuEntry::Item(id) => {
                    let item = MenuItem::with_id(
                        app,
                        id,
                        strings.get(&format!("menu.{id}")),
                        true,
                        accelerator(defaults.get(id)),
                    )?;
                    submenu.append(&item)?;
                    items.insert(id.to_owned(), MenuItemKind::MenuItem(item));
                }
                MenuEntry::Check(id) => {
                    let item = CheckMenuItem::with_id(
                        app,
                        id,
                        strings.get(&format!("menu.{id}")),
                        true,
                        false,
                        accelerator(defaults.get(id)),
                    )?;
                    submenu.append(&item)?;
                    items.insert(id.to_owned(), MenuItemKind::Check(item));
                }
                MenuEntry::System(s) => submenu.append(&system(app, s)?)?,
                MenuEntry::Separator => submenu.append(&PredefinedMenuItem::separator(app)?)?,
            }
        }
        menus.push(submenu);
    }
    #[cfg(debug_assertions)]
    menus.push(Submenu::with_items(
        app,
        "Developer",
        true,
        &[&MenuItem::with_id(
            app,
            "dev-platform",
            "Platform Test Page",
            true,
            None::<&str>,
        )?],
    )?);
    if let Some(state) = app.try_state::<MenuItems>() {
        *state
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = items;
    }
    let refs: Vec<&dyn IsMenuItem<Wry>> = menus.iter().map(|m| m as &dyn IsMenuItem<Wry>).collect();
    Menu::with_items(app, &refs)
}

/// Enabled and checked per command, from the core.
pub fn apply_state(app: &AppHandle, state: &BTreeMap<String, (bool, bool)>) {
    let Some(items) = app.try_state::<MenuItems>() else {
        return;
    };
    let items = items
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (id, (enabled, checked)) in state {
        match items.get(id) {
            Some(MenuItemKind::MenuItem(i)) => {
                let _ = i.set_enabled(*enabled);
            }
            Some(MenuItemKind::Check(i)) => {
                let _ = i.set_enabled(*enabled);
                let _ = i.set_checked(*checked);
            }
            _ => {}
        }
    }
}

/// The user's shortcuts, from the core. The reserved ones also reach the
/// tab views, which give them to the menu before the page (05 §10).
pub fn apply_shortcuts(app: &AppHandle, keys: &BTreeMap<String, String>) {
    let reserved: Vec<String> = mado_types::menu::RESERVED
        .iter()
        .filter_map(|id| keys.get(*id).cloned())
        .collect();
    mado_engine_webkit::set_reserved_shortcuts(&reserved);
    let Some(items) = app.try_state::<MenuItems>() else {
        return;
    };
    let items = items
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (id, k) in keys {
        let result = match items.get(id) {
            Some(MenuItemKind::MenuItem(i)) => i.set_accelerator(accelerator(Some(k))),
            Some(MenuItemKind::Check(i)) => i.set_accelerator(accelerator(Some(k))),
            _ => Ok(()),
        };
        if let Err(e) = result {
            tracing::warn!(id, keys = %k, error = %e, "a shortcut was not applied");
        }
    }
}
