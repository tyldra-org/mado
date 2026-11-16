//! The last native user gesture (spec 11 §4.1 step 3): a local `NSEvent`
//! monitor records when the person last pressed a mouse button or a key in
//! any of Mado's windows. Fills need one in the last second; a page cannot
//! make one.

use std::sync::atomic::{AtomicI64, Ordering};

use block2::RcBlock;
use objc2_app_kit::{NSEvent, NSEventMask};

static LAST: AtomicI64 = AtomicI64::new(0);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Unix milliseconds of the last mouse-down or key-down; 0 before any.
pub fn last_gesture() -> i64 {
    LAST.load(Ordering::Relaxed)
}

/// Installs the monitor once. Main thread.
pub(crate) fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let mask = NSEventMask::LeftMouseDown
            | NSEventMask::RightMouseDown
            | NSEventMask::OtherMouseDown
            | NSEventMask::KeyDown;
        let block = RcBlock::new(|event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
            LAST.store(now_ms(), Ordering::Relaxed);
            event.as_ptr()
        });
        // SAFETY: the monitor copies the block and returns the event
        // unchanged; the token lives for the process.
        let token = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
        std::mem::forget(token);
    });
}

/// For synthesized test events, which do not pass the monitor.
pub(crate) fn record() {
    LAST.store(now_ms(), Ordering::Relaxed);
}
