//! The Dock tile (spec 01 §1.5): each profile process has its own Dock
//! icon; the badge tells them apart.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

/// What a Dock menu item does: its command ID.
type Choose = Box<dyn Fn(&str)>;

thread_local! {
    static MENU: RefCell<Option<Retained<NSMenu>>> = const { RefCell::new(None) };
    static TARGET: RefCell<Option<Retained<DockTarget>>> = const { RefCell::new(None) };
    static ITEMS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static CHOOSE: RefCell<Option<Choose>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoDockMenuTarget"]
    struct DockTarget;

    impl DockTarget {
        #[unsafe(method(choose:))]
        fn choose(&self, sender: &NSMenuItem) {
            let index = usize::try_from(sender.tag()).unwrap_or(usize::MAX);
            let id = ITEMS.with_borrow(|i| i.get(index).cloned());
            if let Some(id) = id {
                CHOOSE.with_borrow(|c| {
                    if let Some(c) = c {
                        c(&id);
                    }
                });
            }
        }
    }
);

/// `-applicationDockMenu:`, added to the app delegate's class.
extern "C-unwind" fn dock_menu(
    _this: *mut AnyObject,
    _cmd: Sel,
    _app: *mut AnyObject,
) -> *mut NSMenu {
    MENU.with_borrow(|m| {
        m.as_ref()
            .map_or(std::ptr::null_mut(), |m| Retained::as_ptr(m).cast_mut())
    })
}

/// The Dock tile's menu (spec 01 §5.4): `items` as (command ID, title),
/// above the windows AppKit lists; `choose` runs with the ID. Main thread
/// only; call once the app's delegate exists.
pub fn set_menu(items: &[(String, String)], choose: impl Fn(&str) + 'static) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let target: Retained<DockTarget> = {
        let this = DockTarget::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    };
    let menu = NSMenu::new(mtm);
    for (i, (_, title)) in items.iter().enumerate() {
        // SAFETY: a new item with an action our target implements.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(sel!(choose:)),
                &NSString::new(),
            )
        };
        item.setTag(isize::try_from(i).unwrap_or(0));
        // SAFETY: the target lives in TARGET as long as the menu.
        unsafe { item.setTarget(Some(&target)) };
        menu.addItem(&item);
    }
    ITEMS.with_borrow_mut(|v| *v = items.iter().map(|(id, _)| id.clone()).collect());
    CHOOSE.with_borrow_mut(|c| *c = Some(Box::new(choose)));
    MENU.with_borrow_mut(|m| *m = Some(menu));
    TARGET.with_borrow_mut(|t| *t = Some(target));
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else {
        return false;
    };
    // `-class` answers the declared class even when KVO has subclassed it.
    // SAFETY: -class takes no arguments and returns a class.
    let class: *const objc2::runtime::AnyClass = unsafe { msg_send![&*delegate, class] };
    let imp: extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut NSMenu = dock_menu;
    // SAFETY: the implementation matches `-(NSMenu *)applicationDockMenu:(NSApplication *)`
    // and its type encoding; adding fails harmlessly if the class has one.
    unsafe {
        objc2::ffi::class_addMethod(
            class.cast_mut(),
            sel!(applicationDockMenu:),
            std::mem::transmute::<
                extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> *mut NSMenu,
                objc2::runtime::Imp,
            >(imp),
            c"@@:@".as_ptr(),
        )
        .as_bool()
    }
}

/// Shows `text` (the profile's initial) on this process's Dock tile, or
/// clears it. Main thread only; elsewhere it does nothing.
pub fn set_badge(text: Option<&str>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let tile = NSApplication::sharedApplication(mtm).dockTile();
    let label = text.map(NSString::from_str);
    tile.setBadgeLabel(label.as_deref());
}
