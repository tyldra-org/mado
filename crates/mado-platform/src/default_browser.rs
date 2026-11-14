//! Default browser check and set (spec 05, links from other apps). Setting
//! it shows macOS's own confirmation prompt.

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSBundle, NSError, NSString, NSURL};

use crate::PlatformError;
use crate::objc::wait;

fn http_url() -> Result<objc2::rc::Retained<NSURL>, PlatformError> {
    NSURL::URLWithString(&NSString::from_str("https://example.com/"))
        .ok_or_else(|| PlatformError::Failed("bad URL".into()))
}

/// True when https links open in this app.
pub fn is_default() -> Result<bool, PlatformError> {
    let workspace = NSWorkspace::sharedWorkspace();
    let handler = workspace.URLForApplicationToOpenURL(&*http_url()?);
    let ours = NSBundle::mainBundle().bundleURL();
    Ok(handler
        .is_some_and(|h| h.path().map(|p| p.to_string()) == ours.path().map(|p| p.to_string())))
}

/// Asks macOS to make this app the handler for `http` and `https`. macOS
/// shows its prompt; the result is whether the user agreed.
pub fn make_default() -> Result<(), PlatformError> {
    let workspace = NSWorkspace::sharedWorkspace();
    let app = NSBundle::mainBundle().bundleURL();
    for scheme in ["http", "https"] {
        let (tx, rx) = mpsc::channel();
        let done = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: nil or a live error for the call.
            let error = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
            let _ = tx.send(error);
        });
        workspace.setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(
            &app,
            &NSString::from_str(scheme),
            Some(&done),
        );
        if let Some(error) = wait(&rx, "NSWorkspace", Duration::from_secs(300))? {
            return Err(PlatformError::Failed(error));
        }
    }
    Ok(())
}
