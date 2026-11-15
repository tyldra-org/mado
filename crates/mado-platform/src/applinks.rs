//! Apps that open other apps' links (spec 07 §6): who handles a URL, its
//! name and icon, and the launch.

use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
use objc2_foundation::{NSDictionary, NSFileManager, NSSize, NSString, NSURL};

use crate::PlatformError;

/// The app macOS would open `url` with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handler {
    pub name: String,
    /// A 64 by 64 PNG of the app's icon.
    pub icon_png: Option<Vec<u8>>,
}

fn ns_url(url: &str) -> Result<objc2::rc::Retained<NSURL>, PlatformError> {
    NSURL::URLWithString(&NSString::from_str(url))
        .ok_or_else(|| PlatformError::Failed("bad URL".into()))
}

/// The app that opens `url`, or `None` when no app does.
pub fn handler(url: &str) -> Result<Option<Handler>, PlatformError> {
    let workspace = NSWorkspace::sharedWorkspace();
    let Some(app) = workspace.URLForApplicationToOpenURL(&*ns_url(url)?) else {
        return Ok(None);
    };
    let path = app
        .path()
        .ok_or_else(|| PlatformError::Failed("the app has no path".into()))?;
    let name = NSFileManager::defaultManager()
        .displayNameAtPath(&path)
        .to_string()
        .trim_end_matches(".app")
        .to_owned();
    let image = workspace.iconForFile(&path);
    image.setSize(NSSize::new(64.0, 64.0));
    let icon_png = image.TIFFRepresentation().and_then(|tiff| {
        let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
        // SAFETY: an empty properties dictionary.
        unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .map(|d| d.to_vec())
    });
    Ok(Some(Handler { name, icon_png }))
}

/// Opens `url` in the app that handles it.
pub fn launch(url: &str) -> Result<(), PlatformError> {
    if NSWorkspace::sharedWorkspace().openURL(&*ns_url(url)?) {
        Ok(())
    } else {
        Err(PlatformError::Failed("no app opened the link".into()))
    }
}
