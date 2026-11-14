//! The `mado:` scheme (spec 01 §4.4): internal pages served from the app's
//! bundle. A request is answered only when the document that makes it is
//! itself a `mado:` page, so a website cannot load internal resources.

use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_foundation::{NSError, NSString, NSURLErrorDomain, NSURLResponse};
use objc2_web_kit::{WKURLSchemeHandler, WKURLSchemeTask, WKWebView};

use crate::convert;

/// A resource for a `mado:` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub mime: String,
    pub body: Vec<u8>,
}

/// Maps a `mado:` URL to a bundled resource; `None` is not found.
pub type SchemeResolver = Arc<dyn Fn(&str) -> Option<Resolved> + Send + Sync>;

pub struct Ivars {
    resolver: SchemeResolver,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoSchemeHandler"]
    #[ivars = Ivars]
    pub struct SchemeHandler;

    unsafe impl NSObjectProtocol for SchemeHandler {}

    unsafe impl WKURLSchemeHandler for SchemeHandler {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start(&self, _view: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            // SAFETY: reading the task's request on the main thread.
            let request = unsafe { task.request() };
            let url = convert::url_string(request.URL());
            let document = convert::url_string(request.mainDocumentURL());
            let from_mado = document.is_empty() || document.starts_with("mado:");
            let found = from_mado.then(|| (self.ivars().resolver)(&url)).flatten();
            match (found, request.URL()) {
                (Some(r), Some(ns_url)) => {
                    let response =
                        NSURLResponse::initWithURL_MIMEType_expectedContentLength_textEncodingName(
                            NSURLResponse::alloc(),
                            &ns_url,
                            Some(&NSString::from_str(&r.mime)),
                            isize::try_from(r.body.len()).unwrap_or(-1),
                            Some(&NSString::from_str("utf-8")),
                        );
                    // SAFETY: answering a started task once, on the main thread.
                    unsafe {
                        task.didReceiveResponse(&response);
                        task.didReceiveData(&convert::data(&r.body));
                        task.didFinish();
                    }
                }
                _ => {
                    // SAFETY: NSURLErrorDomain is a constant; -1100 is "file does not exist".
                    let error = unsafe {
                        NSError::errorWithDomain_code_userInfo(NSURLErrorDomain, -1100, None)
                    };
                    // SAFETY: failing a started task once.
                    unsafe { task.didFailWithError(&error) };
                }
            }
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop(&self, _view: &WKWebView, _task: &ProtocolObject<dyn WKURLSchemeTask>) {}
    }
);

impl SchemeHandler {
    pub fn new(mtm: MainThreadMarker, resolver: SchemeResolver) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { resolver });
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}
