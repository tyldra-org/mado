//! OS authentication (spec 06 §5, 08 §6): Touch ID, falling back to the
//! account password, through `LAContext`. Success gives a one-time token
//! the core accepts for 60 seconds.

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2_foundation::{NSError, NSString};

use crate::PlatformError;
use crate::objc::{class, wait};

/// LAPolicyDeviceOwnerAuthentication: biometrics, else the password.
const DEVICE_OWNER_AUTHENTICATION: isize = 2;

/// Asks the user to authenticate for `reason` ("Mado wants to …"). Blocks
/// until they answer; call it off the main thread.
pub fn authenticate(reason: &str) -> Result<String, PlatformError> {
    let (tx, rx) = mpsc::channel();
    // SAFETY: LAContext is a plain class; evaluatePolicy copies the block and
    // calls it once on a private queue.
    unsafe {
        let context: Retained<AnyObject> = msg_send![class(c"LAContext")?, new];
        let reply = RcBlock::new(move |ok: Bool, error: *mut NSError| {
            let error = error.as_ref().map(|e| e.localizedDescription().to_string());
            let _ = tx.send((ok.as_bool(), error));
        });
        let () = msg_send![
            &*context,
            evaluatePolicy: DEVICE_OWNER_AUTHENTICATION,
            localizedReason: &*NSString::from_str(reason),
            reply: &*reply
        ];
    }
    match wait(&rx, "LocalAuthentication", Duration::from_secs(300))? {
        (true, _) => token(),
        (false, error) => Err(PlatformError::Failed(
            error.unwrap_or_else(|| "authentication failed".into()),
        )),
    }
}

/// 32 random bytes as hex.
pub fn token() -> Result<String, PlatformError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| PlatformError::Failed(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Whether Touch ID or a password can be asked for at all.
pub fn available() -> bool {
    let Ok(cls) = class(c"LAContext") else {
        return false;
    };
    // SAFETY: canEvaluatePolicy:error: takes a policy and an optional error out-pointer.
    unsafe {
        let context: Retained<AnyObject> = msg_send![cls, new];
        let ok: Bool = msg_send![&*context, canEvaluatePolicy: DEVICE_OWNER_AUTHENTICATION, error: std::ptr::null_mut::<*mut NSError>()];
        ok.as_bool()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokens_are_random_and_long() {
        let a = super::token().expect("random");
        assert_eq!(a.len(), 64);
        assert_ne!(a, super::token().expect("random"));
    }
}
