//! Where agent tabs render (spec 13 §7.3, M0 experiment 12): a window that
//! is fully transparent, ignores the mouse, floats above normal windows,
//! never becomes key and stays out of Mission Control and the Window menu,
//! on the screen of the person's window. Its pages report `visible`, get
//! animation frames and unthrottled timers, and never take focus.

use std::cell::RefCell;
use std::collections::BTreeSet;

use mado_types::TabId;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};

thread_local! {
    static HOST: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
    /// Tabs rendering in the host window.
    static TABS: RefCell<BTreeSet<TabId>> = const { RefCell::new(BTreeSet::new()) };
}

/// The host window's content view, made on first use the size of `near`
/// and on its screen.
pub fn content(mtm: MainThreadMarker, near: Option<&NSView>) -> Retained<NSView> {
    let window = HOST.with_borrow_mut(|h| {
        if let Some(w) = h.as_ref() {
            return w.clone();
        }
        let frame = near.and_then(|v| v.window()).map_or_else(
            || {
                objc2_foundation::NSRect::new(
                    objc2_foundation::NSPoint::new(0.0, 0.0),
                    objc2_foundation::NSSize::new(1280.0, 800.0),
                )
            },
            |w| w.frame(),
        );
        // SAFETY: a new borderless window on the main thread.
        let w = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: plain window properties on the main thread; the window is
        // ours and lives as long as the app.
        unsafe { w.setReleasedWhenClosed(false) };
        w.setAlphaValue(0.0);
        w.setIgnoresMouseEvents(true);
        // NSFloatingWindowLevel: nothing occludes it, so pages stay visible.
        w.setLevel(3);
        w.setExcludedFromWindowsMenu(true);
        w.setCollectionBehavior(
            NSWindowCollectionBehavior::Transient
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::Stationary,
        );
        w.orderFrontRegardless();
        *h = Some(w.clone());
        w
    });
    window.contentView().unwrap_or_else(|| NSView::new(mtm))
}

pub fn add(tab: TabId) {
    TABS.with_borrow_mut(|t| t.insert(tab));
}

/// Whether `tab` renders in the host window; forgets it when `take`.
pub fn hosted(tab: TabId, take: bool) -> bool {
    TABS.with_borrow_mut(|t| {
        if take {
            t.remove(&tab)
        } else {
            t.contains(&tab)
        }
    })
}

/// Whether the host window has its properties (for tests and logs).
pub fn host_state() -> Option<serde_json::Value> {
    HOST.with_borrow(|h| {
        h.as_ref().map(|w| {
            serde_json::json!({
                "alpha": w.alphaValue(),
                "ignoresMouse": w.ignoresMouseEvents(),
                "isKey": w.isKeyWindow(),
                "level": w.level(),
                "visible": w.isVisible(),
            })
        })
    })
}
