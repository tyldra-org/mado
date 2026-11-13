//! WKDownload for the downloads experiment (12 §7): start a download, cancel
//! it with resume data, and resume from that data, in this process or a new
//! one.

use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::mpsc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSData, NSError, NSString, NSURL, NSURLRequest, NSURLResponse};
use objc2_web_kit::{WKDownload, WKDownloadDelegate};

use crate::macos::{TabId, main_thread, with_tab};

/// What the download delegate heard.
#[derive(Debug)]
pub enum Event {
    Destination,
    Finished,
    Failed { error: String, resume_data: bool },
}

pub struct Ivars {
    dest: PathBuf,
    events: mpsc::Sender<Event>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoProtoDownloadDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl WKDownloadDelegate for Delegate {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn decide(
            &self,
            _download: &WKDownload,
            _response: &NSURLResponse,
            _name: &NSString,
            handler: &block2::DynBlock<dyn Fn(*mut NSURL)>,
        ) {
            let path = NSString::from_str(&self.ivars().dest.to_string_lossy());
            let url = NSURL::fileURLWithPath(&path);
            let _ = self.ivars().events.send(Event::Destination);
            handler.call((Retained::as_ptr(&url).cast_mut(),));
        }

        #[unsafe(method(downloadDidFinish:))]
        fn finished(&self, _download: &WKDownload) {
            let _ = self.ivars().events.send(Event::Finished);
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn failed(&self, _download: &WKDownload, error: &NSError, resume_data: Option<&NSData>) {
            let _ = self.ivars().events.send(Event::Failed {
                error: error.localizedDescription().to_string(),
                resume_data: resume_data.is_some(),
            });
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, dest: PathBuf, events: mpsc::Sender<Event>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { dest, events });
        // SAFETY: NSObject's designated initializer takes no arguments.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    /// Downloads in flight and their delegates, kept alive until the run ends.
    static LIVE: RefCell<Vec<(Retained<WKDownload>, Retained<Delegate>)>> = const { RefCell::new(Vec::new()) };
}

fn adopt(download: NonNull<WKDownload>, delegate: &Retained<Delegate>) {
    // SAFETY: WebKit hands the completion handler a live download.
    let Some(download) = (unsafe { Retained::retain(download.as_ptr()) }) else {
        return;
    };
    // SAFETY: the delegate lives in LIVE as long as the download.
    unsafe { download.setDelegate(Some(ProtocolObject::from_ref(&**delegate))) };
    LIVE.with_borrow_mut(|live| live.push((download, delegate.clone())));
}

/// Downloads `url` in the tab's context to `dest`.
pub fn start(tab: TabId, url: &str, dest: PathBuf) -> Result<mpsc::Receiver<Event>, String> {
    let mtm = main_thread()?;
    let (tx, rx) = mpsc::channel();
    let delegate = Delegate::new(mtm, dest, tx);
    let url = NSURL::URLWithString(&NSString::from_str(url)).ok_or("not a URL")?;
    let request = NSURLRequest::requestWithURL(&url);
    let block = RcBlock::new(move |download: NonNull<WKDownload>| adopt(download, &delegate));
    // SAFETY: the request and block outlive the call; WebKit copies the block.
    with_tab(tab, |view| unsafe {
        view.startDownloadUsingRequest_completionHandler(&request, &block)
    })?;
    Ok(rx)
}

/// Resumes a download from `cancel`'s resume data; `dest` is used only if
/// WebKit asks for a destination again.
pub fn resume(tab: TabId, data: &[u8], dest: PathBuf) -> Result<mpsc::Receiver<Event>, String> {
    let mtm = main_thread()?;
    let (tx, rx) = mpsc::channel();
    let delegate = Delegate::new(mtm, dest, tx);
    let data = NSData::with_bytes(data);
    let block = RcBlock::new(move |download: NonNull<WKDownload>| adopt(download, &delegate));
    // SAFETY: as in `start`.
    with_tab(tab, |view| unsafe {
        view.resumeDownloadFromResumeData_completionHandler(&data, &block)
    })?;
    Ok(rx)
}

/// Cancels the newest download and returns its resume data, if any.
pub fn cancel_last() -> Result<mpsc::Receiver<Option<Vec<u8>>>, String> {
    main_thread()?;
    let download = LIVE
        .with_borrow(|live| live.last().map(|(d, _)| d.clone()))
        .ok_or("no download in flight")?;
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |data: *mut NSData| {
        // SAFETY: nil or live resume data for the duration of the call.
        let bytes = unsafe { data.as_ref() }.map(|d| d.to_vec());
        let _ = tx.send(bytes);
    });
    // SAFETY: the block lives until WebKit has copied it.
    unsafe { download.cancel(Some(&block)) };
    Ok(rx)
}
