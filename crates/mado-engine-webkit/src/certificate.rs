//! The certificate viewer (spec 08 §3): macOS's `SFCertificatePanel` as a
//! sheet on the tab's window, with the page's server trust, or the trust of
//! the load that failed when the tab shows a certificate error.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::c_void;

use mado_engine::EngineError;
use mado_types::TabId;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::WKWebView;

#[link(name = "SecurityInterface", kind = "framework")]
unsafe extern "C" {}

thread_local! {
    /// The server trust of each tab's last load that failed its
    /// certificate check.
    static FAILED: RefCell<BTreeMap<TabId, Retained<AnyObject>>> = const { RefCell::new(BTreeMap::new()) };
}

/// Keeps the trust a failed load carries, for "View certificate".
pub fn remember_failure(tab: TabId, error: &NSError) {
    let trust = error
        .userInfo()
        .objectForKey(&NSString::from_str("NSURLErrorFailingURLPeerTrustErrorKey"));
    if let Some(trust) = trust {
        FAILED.with_borrow_mut(|f| f.insert(tab, trust));
    }
}

pub fn forget(tab: TabId) {
    FAILED.with_borrow_mut(|f| f.remove(&tab));
}

/// Shows the certificate as a sheet on the view's window.
pub fn show(tab: TabId, view: &WKWebView) -> Result<(), EngineError> {
    // SAFETY: `serverTrust` is a SecTrustRef or NULL, owned by the view.
    let live: *mut c_void = unsafe { msg_send![view, serverTrust] };
    let failed = FAILED.with_borrow(|f| f.get(&tab).cloned());
    let trust: *mut c_void = if live.is_null() {
        failed.as_ref().map_or(std::ptr::null_mut(), |t| {
            Retained::as_ptr(t).cast_mut().cast()
        })
    } else {
        live
    };
    if trust.is_null() {
        return Err(EngineError::Failed("the page has no certificate".into()));
    }
    let window = view
        .window()
        .ok_or_else(|| EngineError::Failed("the tab is in no window".into()))?;
    let class = AnyClass::get(c"SFCertificatePanel")
        .ok_or(EngineError::Unsupported("SecurityInterface is missing"))?;
    // SAFETY: the shared panel, shown as a sheet with a live trust and
    // window; no delegate, so no callback.
    unsafe {
        let panel: *mut AnyObject = msg_send![class, sharedCertificatePanel];
        let Some(panel) = panel.as_ref() else {
            return Err(EngineError::Failed("no certificate panel".into()));
        };
        let _: () = msg_send![
            panel,
            beginSheetForWindow: &*window,
            modalDelegate: std::ptr::null::<AnyObject>(),
            didEndSelector: None::<objc2::runtime::Sel>,
            contextInfo: std::ptr::null_mut::<c_void>(),
            trust: trust,
            showGroup: Bool::YES
        ];
    }
    drop(failed);
    Ok(())
}
