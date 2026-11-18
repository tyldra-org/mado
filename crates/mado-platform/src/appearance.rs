//! The app's appearance (decisions/09-ui.md §1): `appearance.theme` sets
//! NSApp's appearance, so Mado's webviews, its pages' `prefers-color-scheme`,
//! menus and sheets all follow the theme; `system` follows macOS.

/// Sets the app's appearance: `light`, `dark` or `system`. Main thread.
pub fn set(theme: &str) {
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    // SAFETY: the appearance names are AppKit's constants; main thread.
    unsafe {
        let named = match theme {
            "light" => {
                objc2_app_kit::NSAppearance::appearanceNamed(objc2_app_kit::NSAppearanceNameAqua)
            }
            "dark" => objc2_app_kit::NSAppearance::appearanceNamed(
                objc2_app_kit::NSAppearanceNameDarkAqua,
            ),
            _ => None,
        };
        app.setAppearance(named.as_deref());
    }
}
