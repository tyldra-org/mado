//! The Dock tile (spec 01 §1.5): each profile process has its own Dock
//! icon; the badge tells them apart.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;

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
