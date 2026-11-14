//! Open at login (`general.openAtLogin`) through `SMAppService.mainApp`.
//! Works only from `Mado.app`.

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2_foundation::NSError;

use crate::PlatformError;
use crate::objc::{bundled, class};

/// SMAppServiceStatusEnabled.
const ENABLED: isize = 1;

fn service() -> Result<Retained<AnyObject>, PlatformError> {
    if !bundled() {
        return Err(PlatformError::NotBundled);
    }
    // SAFETY: mainAppService returns the shared service for this app.
    Ok(unsafe { msg_send![class(c"SMAppService")?, mainAppService] })
}

pub fn enabled() -> Result<bool, PlatformError> {
    // SAFETY: status is an NSInteger property.
    let status: isize = unsafe { msg_send![&*service()?, status] };
    Ok(status == ENABLED)
}

pub fn set(on: bool) -> Result<(), PlatformError> {
    let service = service()?;
    let mut error: *mut NSError = std::ptr::null_mut();
    // SAFETY: registerAndReturnError:/unregisterAndReturnError: take an
    // error out-pointer and return BOOL.
    let ok: Bool = unsafe {
        if on {
            msg_send![&*service, registerAndReturnError: &mut error]
        } else {
            msg_send![&*service, unregisterAndReturnError: &mut error]
        }
    };
    if ok.as_bool() {
        return Ok(());
    }
    // SAFETY: on failure the error is nil or a live NSError.
    let message = unsafe { error.as_ref() }.map_or_else(
        || "login item change failed".to_owned(),
        |e| e.localizedDescription().to_string(),
    );
    Err(PlatformError::Failed(message))
}
