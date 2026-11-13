//! Experiment 15's native side: a `WKUIDelegate` that records every
//! permission request WebKit makes and denies it (so the OS never prompts),
//! and the macOS permission queries 07 §4 names, none of which prompts.
//! Notifications are posted with provisional authorization: they go quietly
//! to Notification Center, with no banner and no prompt, and are removed.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSArray, NSError, NSString};
use objc2_web_kit::{WKUIDelegate, WKWebView};
use serde_json::{Value, json};

use crate::macos::{TabId, main_thread, with_tab};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    // <CoreGraphics/CGWindow.h>: true when screen capture is allowed. Never prompts.
    fn CGPreflightScreenCaptureAccess() -> bool;
}
// Classes looked up by name below; linking loads them.
#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}
#[link(name = "CoreLocation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Speech", kind = "framework")]
unsafe extern "C" {}
#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}

/// Permission requests WebKit made, in order: the selector and its details.
pub static ASKED: Mutex<Vec<Value>> = Mutex::new(Vec::new());

fn record(selector: &str, details: Value) {
    // Also on stderr, so a run the OS ends still shows what was asked.
    eprintln!("mado-proto: delegate asked {selector} {details}");
    if let Ok(mut asked) = ASKED.lock() {
        asked.push(json!({ "selector": selector, "details": details }));
    }
}

fn host(origin: &AnyObject) -> String {
    // SAFETY: WKSecurityOrigin.host is an NSString.
    let host: Option<Retained<NSString>> = unsafe { msg_send![origin, host] };
    host.map(|h| h.to_string()).unwrap_or_default()
}

/// `WKPermissionDecision`: 0 prompt, 1 grant, 2 deny.
const DENY: isize = 2;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoProtoUIDelegate"]
    struct UiDelegate;

    unsafe impl NSObjectProtocol for UiDelegate {}

    unsafe impl WKUIDelegate for UiDelegate {}

    impl UiDelegate {
        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media(
            &self,
            _view: &WKWebView,
            origin: &AnyObject,
            _frame: &AnyObject,
            kind: isize,
            handler: &DynBlock<dyn Fn(isize)>,
        ) {
            // WKMediaCaptureType: 0 camera, 1 microphone, 2 both.
            record(
                "webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:",
                json!({ "origin": host(origin), "type": kind }),
            );
            handler.call((DENY,));
        }

        #[unsafe(method(webView:requestGeolocationPermissionForOrigin:initiatedByFrame:decisionHandler:))]
        fn geolocation(
            &self,
            _view: &WKWebView,
            origin: &AnyObject,
            _frame: &AnyObject,
            handler: &DynBlock<dyn Fn(isize)>,
        ) {
            record(
                "webView:requestGeolocationPermissionForOrigin:initiatedByFrame:decisionHandler:",
                json!({ "origin": host(origin) }),
            );
            handler.call((DENY,));
        }

        #[unsafe(method(_webView:requestDisplayCapturePermissionForOrigin:initiatedByFrame:withSystemAudio:decisionHandler:))]
        fn display(
            &self,
            _view: &WKWebView,
            origin: &AnyObject,
            _frame: &AnyObject,
            system_audio: Bool,
            handler: &DynBlock<dyn Fn(isize)>,
        ) {
            // WKDisplayCapturePermissionDecision: 0 deny, 1 screen, 2 window.
            record(
                "_webView:requestDisplayCapturePermissionForOrigin:initiatedByFrame:withSystemAudio:decisionHandler:",
                json!({ "origin": host(origin), "systemAudio": system_audio.as_bool() }),
            );
            handler.call((0,));
        }

        #[unsafe(method(_webView:requestSpeechRecognitionPermissionForOrigin:decisionHandler:))]
        fn speech(&self, _view: &WKWebView, origin: &AnyObject, handler: &DynBlock<dyn Fn(Bool)>) {
            record(
                "_webView:requestSpeechRecognitionPermissionForOrigin:decisionHandler:",
                json!({ "origin": host(origin) }),
            );
            handler.call((Bool::NO,));
        }

        #[unsafe(method(_webView:requestNotificationPermissionForSecurityOrigin:decisionHandler:))]
        fn notification(
            &self,
            _view: &WKWebView,
            origin: &AnyObject,
            handler: &DynBlock<dyn Fn(Bool)>,
        ) {
            record(
                "_webView:requestNotificationPermissionForSecurityOrigin:decisionHandler:",
                json!({ "origin": host(origin) }),
            );
            handler.call((Bool::NO,));
        }
    }
);

impl UiDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's designated initializer takes no arguments.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    static DELEGATES: RefCell<Vec<Retained<UiDelegate>>> = const { RefCell::new(Vec::new()) };
}

/// Makes the tab's permission requests go to the recording delegate.
pub fn watch(tab: TabId) -> Result<(), String> {
    let delegate = UiDelegate::new(main_thread()?);
    // SAFETY: the delegate lives in DELEGATES for the rest of the run.
    with_tab(tab, |view| unsafe {
        view.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)))
    })?;
    DELEGATES.with_borrow_mut(|d| d.push(delegate));
    Ok(())
}

fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass, String> {
    AnyClass::get(name).ok_or_else(|| format!("no class {}", name.to_string_lossy()))
}

/// The OS permission states 07 §4 reads before asking. None of these prompts.
pub fn os_states() -> Result<Value, String> {
    // SAFETY: class methods and properties documented to return these
    // enum values; CLLocationManager is created and queried on the main thread.
    unsafe {
        let mic: isize = msg_send![class(c"AVCaptureDevice")?, authorizationStatusForMediaType: objc2_foundation::ns_string!("soun")];
        let camera: isize = msg_send![class(c"AVCaptureDevice")?, authorizationStatusForMediaType: objc2_foundation::ns_string!("vide")];
        let speech: isize = msg_send![class(c"SFSpeechRecognizer")?, authorizationStatus];
        let manager: Retained<AnyObject> = msg_send![class(c"CLLocationManager")?, new];
        let location: i32 = msg_send![&*manager, authorizationStatus];
        let services: Bool = msg_send![class(c"CLLocationManager")?, locationServicesEnabled];
        Ok(json!({
            "microphone": mic, "camera": camera, "speech": speech,
            "location": location, "locationServicesEnabled": services.as_bool(),
            "screen": CGPreflightScreenCaptureAccess(),
            "legend": "AVAuthorizationStatus / SFSpeechRecognizerAuthorizationStatus / CLAuthorizationStatus: 0 not determined, 1 restricted, 2 denied, 3 authorized (location 3 = always)",
        }))
    }
}

fn center() -> Result<Retained<AnyObject>, String> {
    let cls = class(c"UNUserNotificationCenter")?;
    objc2::exception::catch(|| {
        // SAFETY: returns the app's notification center; throws (caught)
        // when the process has no bundle.
        let c: Option<Retained<AnyObject>> = unsafe { msg_send![cls, currentNotificationCenter] };
        c
    })
    .map_err(|e| format!("currentNotificationCenter threw: {e:?}"))?
    .ok_or_else(|| "no notification center".into())
}

fn settle<T: Send + 'static>(rx: &mpsc::Receiver<T>) -> Result<T, String> {
    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|_| "UserNotifications never answered".to_owned())
}

/// `UNAuthorizationStatus` of this app: 0 not determined, 1 denied,
/// 2 authorized, 3 provisional, 4 ephemeral. Call off the main thread.
pub fn notification_status() -> Result<isize, String> {
    let center = center()?;
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |settings: NonNull<AnyObject>| {
        // SAFETY: a live UNNotificationSettings for the call.
        let status: isize = unsafe { msg_send![settings.as_ref(), authorizationStatus] };
        let _ = tx.send(status);
    });
    // SAFETY: the block is copied by the center.
    let () = unsafe { msg_send![&*center, getNotificationSettingsWithCompletionHandler: &*block] };
    settle(&rx)
}

/// Asks for provisional authorization (alerts), which never prompts.
pub fn request_provisional() -> Result<Value, String> {
    let center = center()?;
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |granted: Bool, error: *mut NSError| {
        // SAFETY: nil or a live error for the call.
        let error = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
        let _ = tx.send(json!({ "granted": granted.as_bool(), "error": error }));
    });
    // UNAuthorizationOptionAlert (4) | UNAuthorizationOptionProvisional (64).
    let options: usize = 4 | 64;
    // SAFETY: as above.
    let () = unsafe {
        msg_send![&*center, requestAuthorizationWithOptions: options, completionHandler: &*block]
    };
    settle(&rx)
}

/// Posts a notification now; returns the error, if any.
pub fn post(id: &str, title: &str, body: &str) -> Result<Option<String>, String> {
    let center = center()?;
    let (tx, rx) = mpsc::channel();
    // SAFETY: UNMutableNotificationContent and UNNotificationRequest are
    // plain value classes; the strings live across the calls.
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
            let error = error.as_ref().map(|e| e.localizedDescription().to_string());
            let _ = tx.send(error);
        });
        let () =
            msg_send![&*center, addNotificationRequest: &*request, withCompletionHandler: &*block];
    }
    settle(&rx)
}

/// Identifiers of this app's notifications in Notification Center.
pub fn delivered() -> Result<Vec<String>, String> {
    let center = center()?;
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |list: NonNull<NSArray<AnyObject>>| {
        // SAFETY: an array of live UNNotification objects for the call.
        let ids = unsafe { list.as_ref() }
            .iter()
            .map(|n| unsafe {
                let request: Retained<AnyObject> = msg_send![&*n, request];
                let id: Retained<NSString> = msg_send![&*request, identifier];
                id.to_string()
            })
            .collect::<Vec<_>>();
        let _ = tx.send(ids);
    });
    // SAFETY: as above.
    let () =
        unsafe { msg_send![&*center, getDeliveredNotificationsWithCompletionHandler: &*block] };
    settle(&rx)
}

/// Removes this app's delivered notification `id`.
pub fn remove(id: &str) -> Result<(), String> {
    let center = center()?;
    let ids = NSArray::from_retained_slice(&[NSString::from_str(id)]);
    // SAFETY: an array of identifiers.
    let () = unsafe { msg_send![&*center, removeDeliveredNotificationsWithIdentifiers: &*ids] };
    Ok(())
}
