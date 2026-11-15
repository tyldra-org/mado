//! The page context menu on WebKit (spec 07 §8, 02 §4): `MadoWebView`
//! replaces WebKit's menu with Mado's in `willOpenMenu:withEvent:`,
//! reusing WebKit's own items for what WebKit does itself.

use std::cell::RefCell;
use std::collections::BTreeMap;

use mado_engine::EngineEvent;
use mado_engine::event::{ContextAction, ContextData, MediaKind};
use mado_engine::menu::{MenuEntry, MenuEnv, build, engine_runs};
use mado_engine::page::ContextTarget;
use mado_types::TabId;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{
    NSEvent, NSMenu, NSMenuItem, NSResponder, NSUserInterfaceItemIdentification, NSView,
};
use objc2_foundation::NSString;
use objc2_web_kit::WKWebView;

/// Turns an `en.json` key into the label shown.
pub type Labels = std::sync::Arc<dyn Fn(&str) -> String + Send + Sync>;

/// The open menu: its tab, view, context and the actions by item tag.
type Open = (TabId, Retained<WKWebView>, ContextData, Vec<ContextAction>);

thread_local! {
    static LABELS: RefCell<Option<Labels>> = const { RefCell::new(None) };
    /// What the `context` page script last reported, per tab.
    static TARGETS: RefCell<BTreeMap<TabId, ContextTarget>> = const { RefCell::new(BTreeMap::new()) };
    /// The open menu.
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
    static TARGET: RefCell<Option<Retained<MenuTarget>>> = const { RefCell::new(None) };
}

pub fn set_labels(labels: Labels) {
    LABELS.with_borrow_mut(|l| *l = Some(labels));
}

pub fn remember_target(tab: TabId, target: &ContextTarget) {
    TARGETS.with_borrow_mut(|t| t.insert(tab, target.clone()));
}

pub fn forget(tab: TabId) {
    TARGETS.with_borrow_mut(|t| t.remove(&tab));
}

define_class!(
    #[unsafe(super(WKWebView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoWebView"]
    #[ivars = TabId]
    pub struct MadoWebView;

    impl MadoWebView {
        #[unsafe(method(willOpenMenu:withEvent:))]
        fn will_open_menu(&self, menu: &NSMenu, _event: &NSEvent) {
            let tab = *self.ivars();
            replace(tab, self, menu);
        }
    }
);

impl MadoWebView {
    /// A web view for `tab` with Mado's context menu.
    pub fn create(
        mtm: MainThreadMarker,
        tab: TabId,
        frame: objc2_foundation::NSRect,
        config: &objc2_web_kit::WKWebViewConfiguration,
    ) -> Retained<WKWebView> {
        let this = Self::alloc(mtm).set_ivars(tab);
        // SAFETY: WKWebView's designated initializer.
        let view: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: frame, configuration: config] };
        Retained::into_super(view)
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoMenuTarget"]
    pub struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(chosen:))]
        fn chosen(&self, item: &NSMenuItem) {
            chosen(item.tag());
        }
    }
);

fn target(mtm: MainThreadMarker) -> Retained<MenuTarget> {
    TARGET.with_borrow_mut(|t| {
        t.get_or_insert_with(|| {
            let this = MenuTarget::alloc(mtm).set_ivars(());
            // SAFETY: NSObject's designated initializer.
            unsafe { msg_send![super(this), init] }
        })
        .clone()
    })
}

/// WebKit's own item for an action WebKit runs, by identifier.
fn webkit_identifier(action: &ContextAction) -> Option<&'static str> {
    use ContextAction as A;
    Some(match action {
        A::SaveLinkAs => "WKMenuItemIdentifierDownloadLinkedFile",
        A::SaveImageAs => "WKMenuItemIdentifierDownloadImage",
        A::CopyImage => "WKMenuItemIdentifierCopyImage",
        A::SaveVideoAs => "WKMenuItemIdentifierDownloadMedia",
        A::Copy => "WKMenuItemIdentifierCopy",
        A::LookUp => "WKMenuItemIdentifierLookUp",
        A::Paste => "WKMenuItemIdentifierPaste",
        A::Back => "WKMenuItemIdentifierGoBack",
        A::Forward => "WKMenuItemIdentifierGoForward",
        A::Reload => "WKMenuItemIdentifierReload",
        A::Inspect => "WKMenuItemIdentifierInspectElement",
        A::MediaShowControls => "WKMenuItemIdentifierShowHideMediaControls",
        A::MediaPictureInPicture => "WKMenuItemIdentifierToggleVideoPictureInPicture",
        A::MediaPlayPause => "WKMenuItemIdentifierPlayPause",
        A::MediaMute => "WKMenuItemIdentifierToggleMute",
        A::MediaLoop => "WKMenuItemIdentifierToggleLoop",
        _ => return None,
    })
}

/// The responder action for edit commands WebKit answers.
fn responder_action(action: &ContextAction) -> Option<Sel> {
    use ContextAction as A;
    Some(match action {
        A::Undo => sel!(undo:),
        A::Redo => sel!(redo:),
        A::Cut => sel!(cut:),
        A::Copy => sel!(copy:),
        A::Paste => sel!(paste:),
        A::PasteAndMatchStyle => sel!(pasteAsPlainText:),
        A::SelectAll => sel!(selectAll:),
        A::EmojiAndSymbols => sel!(orderFrontCharacterPalette:),
        _ => return None,
    })
}

fn replace(tab: TabId, view: &MadoWebView, menu: &NSMenu) {
    let mtm = MainThreadMarker::from(view);
    let wk: &WKWebView = view;
    let t = TARGETS
        .with_borrow(|t| t.get(&tab).cloned())
        .unwrap_or(ContextTarget {
            link: None,
            link_text: None,
            image: None,
            media: None,
            media_kind: None,
            selection: None,
            editable: false,
        });
    // WebKit's items, by identifier, and its spelling suggestions (the items
    // before the first separator when the target is editable).
    let mut original: BTreeMap<String, Retained<NSMenuItem>> = BTreeMap::new();
    let mut spelling = Vec::new();
    let mut leading = true;
    for item in menu.itemArray().iter() {
        if item.isSeparatorItem() {
            leading = false;
            continue;
        }
        match item.identifier() {
            Some(id) => {
                original.insert(id.to_string(), item.clone());
            }
            None if leading && t.editable => spelling.push(item.clone()),
            None => {}
        }
    }
    // SAFETY: the view's URL on the main thread.
    let page_url = crate::convert::url_string(unsafe { wk.URL() });
    let context = ContextData {
        page_url,
        frame_url: None,
        link_url: t.link.clone(),
        link_text: t.link_text.clone(),
        image_url: t.image.clone(),
        media_url: t.media.clone(),
        media_kind: match t.media_kind.as_deref() {
            Some("video") => Some(MediaKind::Video),
            Some("audio") => Some(MediaKind::Audio),
            _ => None,
        },
        selection: t.selection.clone(),
        editable: t.editable,
        spelling_suggestions: spelling.iter().map(|i| i.title().to_string()).collect(),
    };
    let label = |key: &str| {
        let raw = LABELS
            .with_borrow(|l| l.as_ref().map(|f| f(key)))
            .unwrap_or_else(|| key.to_owned());
        let text: String = context
            .selection
            .as_deref()
            .unwrap_or_default()
            .chars()
            .take(30)
            .collect();
        raw.replace("{text}", &text)
    };
    let target = target(mtm);
    menu.removeAllItems();
    let mut actions = Vec::new();
    for entry in build(&context, &MenuEnv { can_search: true }) {
        let MenuEntry::Item { action, label: key } = entry else {
            if menu.numberOfItems() > 0 {
                menu.addItem(&NSMenuItem::separatorItem(mtm));
            }
            continue;
        };
        let item = if let ContextAction::Spelling(word) = &action {
            spelling
                .iter()
                .find(|i| i.title().to_string() == *word)
                .cloned()
        } else if engine_runs(&action) {
            if let Some(found) = webkit_identifier(&action).and_then(|id| original.get(id)) {
                found.setTitle(&NSString::from_str(&label(key)));
                Some(found.clone())
            } else {
                responder_action(&action).map(|sel| {
                    // SAFETY: a menu item with a first-responder action.
                    unsafe {
                        NSMenuItem::initWithTitle_action_keyEquivalent(
                            NSMenuItem::alloc(mtm),
                            &NSString::from_str(&label(key)),
                            Some(sel),
                            &NSString::new(),
                        )
                    }
                })
            }
        } else {
            // SAFETY: a menu item whose target lives as long as the app.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(&label(key)),
                    Some(sel!(chosen:)),
                    &NSString::new(),
                )
            };
            // SAFETY: as above.
            unsafe { item.setTarget(Some(&target)) };
            item.setTag(isize::try_from(actions.len()).unwrap_or(0));
            actions.push(action);
            Some(item)
        };
        if let Some(item) = item {
            menu.addItem(&item);
        }
    }
    // A separator never ends the menu.
    while menu.numberOfItems() > 0
        && menu
            .itemAtIndex(menu.numberOfItems() - 1)
            .is_some_and(|i| i.isSeparatorItem())
    {
        menu.removeItemAtIndex(menu.numberOfItems() - 1);
    }
    OPEN.with_borrow_mut(|o| *o = Some((tab, wk.retain(), context, actions)));
}

fn chosen(tag: isize) {
    let Some((tab, _view, context, actions)) = OPEN.with_borrow(Clone::clone) else {
        return;
    };
    let Some(action) = usize::try_from(tag)
        .ok()
        .and_then(|i| actions.get(i))
        .cloned()
    else {
        return;
    };
    crate::delegate::send(EngineEvent::ContextMenuChosen {
        tab,
        context,
        action,
    });
}
