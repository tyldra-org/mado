//! Element full screen (spec 08 §5): the "<host> is now full screen ·
//! Press Esc to exit" notice for 4 seconds, again when the pointer reaches
//! the top edge, and Esc handled before any page sees the key.

use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSFont, NSPanel, NSScreen,
    NSStatusWindowLevel, NSTextField, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSTimer};
use objc2_web_kit::{WKFullscreenState, WKWebView};

/// How long the notice stays (08 §5).
const NOTICE_SECONDS: f64 = 4.0;
/// The pointer this close to the screen's top edge shows the notice again.
const TOP_EDGE: f64 = 2.0;
/// Esc's virtual key code.
const ESCAPE: u16 = 53;

struct Active {
    view: Retained<WKWebView>,
    host: String,
    monitor: Option<Retained<AnyObject>>,
    panel: Option<Retained<NSPanel>>,
    timer: Option<Retained<NSTimer>>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
}

/// The notice's text for `host`.
pub fn notice_text(host: &str) -> String {
    crate::menu::label("fullscreen.notice").replace("{host}", host)
}

/// Whether a key event exits full screen: Esc with no modifier but Fn.
pub fn exits(key_code: u16, modifiers: usize) -> bool {
    use objc2_app_kit::NSEventModifierFlags as M;
    let held = M(modifiers) & (M::Command | M::Option | M::Control | M::Shift);
    key_code == ESCAPE && held.is_empty()
}

/// A tab view entered or left element full screen.
pub fn changed(view: &WKWebView) {
    // SAFETY: reading a live view's state on the main thread.
    let state = unsafe { view.fullscreenState() };
    let entering = state != WKFullscreenState::NotInFullscreen;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if !entering {
        if ACTIVE.with_borrow(|a| a.as_ref().is_some_and(|a| &*a.view == view)) {
            stop();
        }
        return;
    }
    if state != WKFullscreenState::InFullscreen
        || ACTIVE.with_borrow(|a| a.as_ref().is_some_and(|a| &*a.view == view))
    {
        return;
    }
    stop();
    // SAFETY: the view's current URL.
    let host = unsafe { view.URL() }
        .and_then(|u| u.host())
        .map(|h| h.to_string())
        .unwrap_or_default();
    ACTIVE.with_borrow_mut(|a| {
        *a = Some(Active {
            view: view.retain(),
            host,
            monitor: monitor(),
            panel: None,
            timer: None,
        });
    });
    show(mtm);
}

/// Exits full screen, as `TabCommand::ExitFullscreen` does.
fn exit() {
    let view = ACTIVE.with_borrow(|a| a.as_ref().map(|a| a.view.clone()));
    if let Some(view) = view {
        // SAFETY: a live view on the main thread.
        unsafe { view.closeAllMediaPresentationsWithCompletionHandler(None) };
    }
}

fn stop() {
    if let Some(a) = ACTIVE.with_borrow_mut(Option::take) {
        if let Some(m) = a.monitor {
            // SAFETY: removing the monitor this module added.
            unsafe { NSEvent::removeMonitor(&m) };
        }
        if let Some(t) = a.timer {
            t.invalidate();
        }
        if let Some(p) = a.panel {
            p.orderOut(None);
        }
    }
}

/// Esc and the pointer at the top edge, seen before any window: a page
/// never receives the Esc that exits.
fn monitor() -> Option<Retained<AnyObject>> {
    let block = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit hands a live event.
        let e = unsafe { event.as_ref() };
        let kind = e.r#type();
        if kind == objc2_app_kit::NSEventType::KeyDown && exits(e.keyCode(), e.modifierFlags().0) {
            exit();
            return std::ptr::null_mut();
        }
        if kind == objc2_app_kit::NSEventType::MouseMoved
            && let Some(mtm) = MainThreadMarker::new()
        {
            let at = NSEvent::mouseLocation();
            let top = NSScreen::mainScreen(mtm).map(|s| {
                let f = s.frame();
                f.origin.y + f.size.height
            });
            let shown = ACTIVE.with_borrow(|a| a.as_ref().is_some_and(|a| a.timer.is_some()));
            if top.is_some_and(|top| at.y >= top - TOP_EDGE) && !shown {
                show(mtm);
            }
        }
        event.as_ptr()
    });
    // SAFETY: a local monitor with a block that returns the event or nil.
    unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::KeyDown | NSEventMask::MouseMoved,
            &block,
        )
    }
}

/// Shows the notice at the top of the screen for [`NOTICE_SECONDS`].
fn show(mtm: MainThreadMarker) {
    let Some(host) = ACTIVE.with_borrow(|a| a.as_ref().map(|a| a.host.clone())) else {
        return;
    };
    let panel = ACTIVE
        .with_borrow(|a| a.as_ref().and_then(|a| a.panel.clone()))
        .unwrap_or_else(|| make_panel(mtm));
    let text = NSString::from_str(&notice_text(&host));
    let label = NSTextField::labelWithString(&text, mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    label.setTextColor(Some(&NSColor::whiteColor()));
    let size = label.fittingSize();
    let (w, h) = (size.width + 32.0, size.height + 16.0);
    label.setFrame(NSRect::new(NSPoint::new(16.0, 8.0), size));
    let screen = NSScreen::mainScreen(mtm)
        .map(|s| s.frame())
        .unwrap_or_default();
    panel.setFrame_display(
        NSRect::new(
            NSPoint::new(
                screen.origin.x + (screen.size.width - w) / 2.0,
                screen.origin.y + screen.size.height - h - 24.0,
            ),
            NSSize::new(w, h),
        ),
        true,
    );
    if let Some(content) = panel.contentView() {
        for v in content.subviews().iter() {
            v.removeFromSuperview();
        }
        content.addSubview(&label);
    }
    panel.orderFrontRegardless();
    let timer = {
        let hide = RcBlock::new(|_: NonNull<NSTimer>| {
            ACTIVE.with_borrow_mut(|a| {
                if let Some(a) = a.as_mut() {
                    a.timer = None;
                    if let Some(p) = &a.panel {
                        p.orderOut(None);
                    }
                }
            });
        });
        // SAFETY: a one-shot main-run-loop timer with a block.
        unsafe {
            NSTimer::scheduledTimerWithTimeInterval_repeats_block(NOTICE_SECONDS, false, &hide)
        }
    };
    ACTIVE.with_borrow_mut(|a| {
        if let Some(a) = a.as_mut() {
            if let Some(old) = a.timer.replace(timer) {
                old.invalidate();
            }
            a.panel = Some(panel);
        }
    });
}

fn make_panel(mtm: MainThreadMarker) -> Retained<NSPanel> {
    // SAFETY: a borderless, non-activating panel that ignores the mouse,
    // above the full-screen window.
    unsafe {
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(10.0, 10.0)),
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        panel.setReleasedWhenClosed(false);
        panel.setLevel(NSStatusWindowLevel);
        panel.setIgnoresMouseEvents(true);
        panel.setOpaque(false);
        panel.setHasShadow(true);
        panel.setBackgroundColor(Some(&NSColor::colorWithWhite_alpha(0.1, 0.85)));
        panel.setCollectionBehavior(
            objc2_app_kit::NSWindowCollectionBehavior::CanJoinAllSpaces
                | objc2_app_kit::NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_plain_escape_exits() {
        use objc2_app_kit::NSEventModifierFlags as M;
        assert!(exits(ESCAPE, 0));
        assert!(exits(ESCAPE, M::Function.0));
        assert!(!exits(ESCAPE, M::Command.0));
        assert!(!exits(12, 0));
    }

    #[test]
    fn the_notice_names_the_host() {
        crate::menu::set_labels(std::sync::Arc::new(|_: &str| {
            "{host} is now full screen · Press Esc to exit".to_owned()
        }));
        assert_eq!(
            notice_text("video.example"),
            "video.example is now full screen · Press Esc to exit"
        );
    }
}
