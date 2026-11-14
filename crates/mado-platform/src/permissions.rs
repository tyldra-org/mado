//! The OS permissions Mado needs before a site may use a device (spec 07
//! §4): status without prompting, and the request that shows the system
//! prompt.

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2_foundation::ns_string;

use crate::PlatformError;
use crate::objc::{class, wait};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OsPermission {
    Camera,
    Microphone,
    Location,
    Screen,
    Speech,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OsStatus {
    /// Never asked.
    NotDetermined,
    /// Parental controls or an MDM profile.
    Restricted,
    Denied,
    Granted,
}

impl OsStatus {
    /// AVAuthorizationStatus, SFSpeechRecognizerAuthorizationStatus and
    /// CLAuthorizationStatus share 0 to 3 (location's 3 and 4 both allow).
    fn from_raw(raw: isize) -> Self {
        match raw {
            0 => Self::NotDetermined,
            1 => Self::Restricted,
            2 => Self::Denied,
            _ => Self::Granted,
        }
    }
}

/// The current status. Never prompts.
pub fn status(permission: OsPermission) -> Result<OsStatus, PlatformError> {
    // SAFETY: class methods documented to return these enum values.
    unsafe {
        let raw: isize = match permission {
            OsPermission::Camera => {
                msg_send![class(c"AVCaptureDevice")?, authorizationStatusForMediaType: ns_string!("vide")]
            }
            OsPermission::Microphone => {
                msg_send![class(c"AVCaptureDevice")?, authorizationStatusForMediaType: ns_string!("soun")]
            }
            OsPermission::Speech => msg_send![class(c"SFSpeechRecognizer")?, authorizationStatus],
            OsPermission::Location => {
                let manager: Retained<AnyObject> = msg_send![class(c"CLLocationManager")?, new];
                let raw: i32 = msg_send![&*manager, authorizationStatus];
                raw as isize
            }
            OsPermission::Screen => {
                return Ok(if CGPreflightScreenCaptureAccess() {
                    OsStatus::Granted
                } else {
                    OsStatus::NotDetermined
                });
            }
        };
        Ok(OsStatus::from_raw(raw))
    }
}

/// Shows the system prompt when the status is not determined, and returns
/// the result. Location's answer arrives through a delegate the core owns
/// from M2; here it returns the status after asking.
pub fn request(permission: OsPermission) -> Result<OsStatus, PlatformError> {
    let within = Duration::from_secs(300);
    match permission {
        OsPermission::Camera | OsPermission::Microphone => {
            let (tx, rx) = mpsc::channel();
            let block = RcBlock::new(move |granted: Bool| {
                let _ = tx.send(granted.as_bool());
            });
            let media = if permission == OsPermission::Camera {
                ns_string!("vide")
            } else {
                ns_string!("soun")
            };
            // SAFETY: the block is copied by AVFoundation and called once.
            unsafe {
                let () = msg_send![class(c"AVCaptureDevice")?, requestAccessForMediaType: media, completionHandler: &*block];
            }
            Ok(if wait(&rx, "AVFoundation", within)? {
                OsStatus::Granted
            } else {
                OsStatus::Denied
            })
        }
        OsPermission::Speech => {
            let (tx, rx) = mpsc::channel();
            let block = RcBlock::new(move |raw: isize| {
                let _ = tx.send(raw);
            });
            // SAFETY: as above.
            unsafe {
                let () = msg_send![class(c"SFSpeechRecognizer")?, requestAuthorization: &*block];
            }
            Ok(OsStatus::from_raw(wait(&rx, "Speech", within)?))
        }
        OsPermission::Location => {
            // SAFETY: requestWhenInUseAuthorization shows the prompt; the
            // manager must outlive it, so it is kept for the process.
            unsafe {
                let manager: Retained<AnyObject> = msg_send![class(c"CLLocationManager")?, new];
                let () = msg_send![&*manager, requestWhenInUseAuthorization];
                std::mem::forget(manager);
            }
            status(OsPermission::Location)
        }
        // SAFETY: a plain C function; it opens System Settings when denied.
        OsPermission::Screen => Ok(if unsafe { CGRequestScreenCaptureAccess() } {
            OsStatus::Granted
        } else {
            OsStatus::Denied
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_read_without_prompting() {
        for p in [
            OsPermission::Camera,
            OsPermission::Microphone,
            OsPermission::Location,
            OsPermission::Screen,
            OsPermission::Speech,
        ] {
            status(p).unwrap_or_else(|e| panic!("{p:?}: {e}"));
        }
    }
}
