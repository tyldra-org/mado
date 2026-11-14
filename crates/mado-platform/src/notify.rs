//! Native notifications (spec 07 §1.1) with a click callback. They need
//! `Mado.app`: UserNotifications refuses a bare binary.

use std::sync::{Mutex, mpsc};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject};
use objc2::{AnyThread, define_class, msg_send};
use objc2_foundation::{NSError, NSObjectProtocol, NSString};

use crate::PlatformError;
use crate::objc::{bundled, class, wait};

type Clicked = Box<dyn Fn(String) + Send>;

/// Called with the notification's ID when the user clicks it.
static ON_CLICK: Mutex<Option<Clicked>> = Mutex::new(None);

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MadoNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    impl Delegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn clicked(&self, _center: &AnyObject, response: &AnyObject, done: &block2::DynBlock<dyn Fn()>) {
            // SAFETY: UNNotificationResponse.notification.request.identifier is an NSString.
            let id: Retained<NSString> = unsafe {
                let notification: Retained<AnyObject> = msg_send![response, notification];
                let request: Retained<AnyObject> = msg_send![&*notification, request];
                msg_send![&*request, identifier]
            };
            if let Ok(guard) = ON_CLICK.lock()
                && let Some(callback) = guard.as_ref()
            {
                callback(id.to_string());
            }
            done.call(());
        }

        /// Show notifications while Mado is in front, too.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _center: &AnyObject, _notification: &AnyObject, done: &block2::DynBlock<dyn Fn(usize)>) {
            // UNNotificationPresentationOptionBanner | List.
            done.call((16 | 8,));
        }
    }
);

fn center() -> Result<Retained<AnyObject>, PlatformError> {
    if !bundled() {
        return Err(PlatformError::NotBundled);
    }
    // SAFETY: the app's notification center.
    let center: Option<Retained<AnyObject>> = unsafe {
        msg_send![
            class(c"UNUserNotificationCenter")?,
            currentNotificationCenter
        ]
    };
    center.ok_or(PlatformError::NotBundled)
}

/// Installs the click callback. Call once at start.
pub fn on_click(callback: impl Fn(String) + Send + 'static) -> Result<(), PlatformError> {
    let center = center()?;
    if let Ok(mut slot) = ON_CLICK.lock() {
        *slot = Some(Box::new(callback));
    }
    // SAFETY: NSObject's designated initializer takes no arguments.
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
    // SAFETY: the center keeps a weak reference, so the delegate is kept
    // for the process.
    unsafe {
        let () = msg_send![&*center, setDelegate: &*delegate];
    }
    std::mem::forget(delegate);
    Ok(())
}

/// Asks for permission to show alerts (the system prompt, once).
pub fn request_permission() -> Result<bool, PlatformError> {
    let center = center()?;
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        let _ = tx.send(granted.as_bool());
    });
    // UNAuthorizationOptionBadge | Sound | Alert.
    let options: usize = 1 | 2 | 4;
    // SAFETY: the block is copied and called once.
    unsafe {
        let () = msg_send![&*center, requestAuthorizationWithOptions: options, completionHandler: &*block];
    }
    wait(&rx, "UserNotifications", Duration::from_secs(300))
}

/// Shows a notification now.
pub fn post(id: &str, title: &str, body: &str) -> Result<(), PlatformError> {
    let center = center()?;
    let (tx, rx) = mpsc::channel();
    // SAFETY: plain value classes; the strings outlive the calls.
    unsafe {
        let content: Retained<AnyObject> = msg_send![class(c"UNMutableNotificationContent")?, new];
        let () = msg_send![&*content, setTitle: &*NSString::from_str(title)];
        let () = msg_send![&*content, setBody: &*NSString::from_str(body)];
        let request: Retained<AnyObject> = msg_send![
            class(c"UNNotificationRequest")?,
            requestWithIdentifier: &*NSString::from_str(id),
            content: &*content,
            trigger: std::ptr::null::<AnyObject>()
        ];
        let block = RcBlock::new(move |error: *mut NSError| {
            let _ = tx.send(error.as_ref().map(|e| e.localizedDescription().to_string()));
        });
        let () =
            msg_send![&*center, addNotificationRequest: &*request, withCompletionHandler: &*block];
    }
    match wait(&rx, "UserNotifications", Duration::from_secs(10))? {
        None => Ok(()),
        Some(error) => Err(PlatformError::Failed(error)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_bare_binary_is_told_it_needs_the_app() {
        assert_eq!(
            super::post("t", "a", "b"),
            Err(crate::PlatformError::NotBundled)
        );
    }
}
