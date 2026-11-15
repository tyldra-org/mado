//! Downloads on WebKit (spec 12 §4.2): `WKDownloadDelegate` asks the core
//! where each file goes, reports progress by observing the download's
//! `NSProgress`, and pauses, cancels and resumes with resume data.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::ptr::NonNull;

use block2::RcBlock;
use mado_engine::event::{
    DownloadCommand, DownloadDestination, DownloadError, EngineDownload, EngineDownloadId,
};
use mado_engine::{EngineEvent, NetError};
use mado_types::TabId;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{NSModalResponseOK, NSSavePanel};
use objc2_foundation::{
    NSData, NSDictionary, NSError, NSKeyValueChangeKey, NSObjectNSKeyValueObserverRegistration,
    NSProgress, NSProgressReporting, NSString, NSURL, NSURLResponse,
};
use objc2_web_kit::{WKDownload, WKDownloadDelegate};

use crate::convert;

struct Running {
    download: Retained<WKDownload>,
    delegate: Retained<DownloadDelegate>,
    progress: Retained<NSProgress>,
}

thread_local! {
    static RUNNING: RefCell<BTreeMap<EngineDownloadId, Running>> = const { RefCell::new(BTreeMap::new()) };
    /// Where each download is being written, for its completion.
    static PATHS: RefCell<BTreeMap<EngineDownloadId, PathBuf>> = const { RefCell::new(BTreeMap::new()) };
}

pub struct Ivars {
    id: EngineDownloadId,
    tab: Option<TabId>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoDownloadDelegate"]
    #[ivars = Ivars]
    pub struct DownloadDelegate;

    unsafe impl NSObjectProtocol for DownloadDelegate {}

    unsafe impl WKDownloadDelegate for DownloadDelegate {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn destination(
            &self,
            download: &WKDownload,
            response: &NSURLResponse,
            suggested: &NSString,
            handler: &block2::DynBlock<dyn Fn(*mut NSURL)>,
        ) {
            let id = self.ivars().id;
            // A resumed download already has its place.
            if let Some(path) = PATHS.with_borrow(|p| p.get(&id).cloned()) {
                let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
                handler.call((Retained::as_ptr(&url).cast_mut(),));
                return;
            }
            // SAFETY: reading the download's request on the main thread.
            let url = unsafe { download.originalRequest() }.map(|r| convert::url_string(r.URL())).unwrap_or_default();
            let total = response.expectedContentLength();
            let info = EngineDownload {
                id,
                url,
                suggested_filename: suggested.to_string(),
                mime: response.MIMEType().map(|m| m.to_string()),
                total: u64::try_from(total).ok().filter(|t| *t > 0),
            };
            let handler = handler.copy();
            let Some(reply) = crate::delegate::park_download(id, handler) else { return };
            crate::delegate::send(EngineEvent::DownloadStarted { tab: self.ivars().tab, download: info, reply });
        }

        #[unsafe(method(downloadDidFinish:))]
        fn finished(&self, _download: &WKDownload) {
            let id = self.ivars().id;
            stop_tracking(id);
            let path = PATHS.with_borrow_mut(|p| p.remove(&id));
            let result = path.ok_or_else(|| DownloadError::File("no destination".into()));
            crate::delegate::send(EngineEvent::DownloadFinished { id, result });
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn failed(&self, _download: &WKDownload, error: &NSError, resume: Option<&NSData>) {
            let id = self.ivars().id;
            stop_tracking(id);
            let result = match resume {
                Some(data) => Err(DownloadError::Interrupted { resume_data: convert::bytes(data) }),
                None if error.code() == -999 => Err(DownloadError::Cancelled),
                None => Err(DownloadError::Network(NetError::from_nsurl_code(convert::error_code(error)))),
            };
            crate::delegate::send(EngineEvent::DownloadFinished { id, result });
        }
    }

    impl DownloadDelegate {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            _key: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>,
            _context: *mut std::ffi::c_void,
        ) {
            let Some(p) = object.and_then(|o| o.downcast_ref::<NSProgress>()) else { return };
            let received = u64::try_from(p.completedUnitCount()).unwrap_or(0);
            let total = u64::try_from(p.totalUnitCount()).ok().filter(|t| *t > 0);
            crate::delegate::send(EngineEvent::DownloadProgress { id: self.ivars().id, received, total });
        }
    }
);

impl DownloadDelegate {
    fn new(mtm: MainThreadMarker, id: EngineDownloadId, tab: Option<TabId>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars { id, tab });
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

fn stop_tracking(id: EngineDownloadId) {
    if let Some(r) = RUNNING.with_borrow_mut(|m| m.remove(&id)) {
        // SAFETY: removing the observer added in `track`.
        unsafe {
            r.progress
                .removeObserver_forKeyPath(&r.delegate, ns_completed())
        };
        let _ = r.download;
    }
}

fn ns_completed() -> &'static NSString {
    objc2_foundation::ns_string!("completedUnitCount")
}

/// Starts following a download WebKit created: its delegate and progress.
pub fn track(
    mtm: MainThreadMarker,
    id: EngineDownloadId,
    tab: Option<TabId>,
    download: &WKDownload,
) {
    let delegate = DownloadDelegate::new(mtm, id, tab);
    // SAFETY: the download holds its delegate weakly; RUNNING keeps it.
    unsafe { download.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    let progress = download.progress();
    // SAFETY: observing a live NSProgress; removed in `stop_tracking`.
    unsafe {
        progress.addObserver_forKeyPath_options_context(
            &delegate,
            ns_completed(),
            objc2_foundation::NSKeyValueObservingOptions::New,
            std::ptr::null_mut(),
        );
    }
    RUNNING.with_borrow_mut(|m| {
        m.insert(
            id,
            Running {
                download: download.retain(),
                delegate,
                progress,
            },
        )
    });
}

/// The core decided where the file goes.
pub fn answer_destination(
    id: EngineDownloadId,
    handler: &RcBlock<dyn Fn(*mut NSURL)>,
    destination: DownloadDestination,
) {
    let path = match destination {
        DownloadDestination::Path(p) => Some(p),
        DownloadDestination::Ask(p) => ask(&p),
        DownloadDestination::Cancel => None,
    };
    match path {
        Some(p) => {
            PATHS.with_borrow_mut(|m| m.insert(id, p.clone()));
            let url = NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()));
            handler.call((Retained::as_ptr(&url).cast_mut(),));
        }
        None => handler.call((std::ptr::null_mut(),)),
    }
}

/// The save panel for `downloads.askWhere`. Returns the chosen path with
/// the temporary suffix kept.
fn ask(suggested: &std::path::Path) -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    let final_path = mado_engine_suffix::strip(suggested);
    let panel = NSSavePanel::savePanel(mtm);
    if let Some(name) = final_path.file_name() {
        panel.setNameFieldStringValue(&NSString::from_str(&name.to_string_lossy()));
    }
    if let Some(dir) = final_path.parent() {
        panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(
            &dir.to_string_lossy(),
        ))));
    }
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let chosen = panel.URL()?.path()?.to_string();
    Some(mado_engine_suffix::add(&PathBuf::from(chosen)))
}

/// The temporary suffix, as the core uses it (`mado_core::downloads`).
mod mado_engine_suffix {
    use std::path::{Path, PathBuf};

    const SUFFIX: &str = ".madodownload";

    pub fn strip(p: &Path) -> PathBuf {
        let s = p.to_string_lossy();
        PathBuf::from(s.strip_suffix(SUFFIX).unwrap_or(&s).to_owned())
    }

    pub fn add(p: &Path) -> PathBuf {
        let mut s = p.as_os_str().to_owned();
        s.push(SUFFIX);
        PathBuf::from(s)
    }
}

/// `Engine::download`: pause (cancel with resume data), cancel, resume.
pub fn command(
    id: EngineDownloadId,
    cmd: DownloadCommand,
    any_view: Option<Retained<objc2_web_kit::WKWebView>>,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    match cmd {
        DownloadCommand::Pause | DownloadCommand::Cancel => {
            let Some(download) = RUNNING.with_borrow(|m| m.get(&id).map(|r| r.download.clone()))
            else {
                return;
            };
            let keep = matches!(cmd, DownloadCommand::Pause);
            let block = RcBlock::new(move |data: *mut NSData| {
                stop_tracking(id);
                // SAFETY: WebKit hands resume data or null.
                let data = unsafe { data.as_ref() };
                let result = match data {
                    Some(d) if keep => Err(DownloadError::Interrupted {
                        resume_data: convert::bytes(d),
                    }),
                    _ => Err(DownloadError::Cancelled),
                };
                crate::delegate::send(EngineEvent::DownloadFinished { id, result });
            });
            // SAFETY: cancelling a running download with a completion block.
            unsafe { download.cancel(Some(&block)) };
        }
        DownloadCommand::Resume { resume_data, path } => {
            PATHS.with_borrow_mut(|m| m.insert(id, path));
            let Some(view) = any_view else {
                crate::delegate::send(EngineEvent::DownloadFinished {
                    id,
                    result: Err(DownloadError::Interrupted { resume_data }),
                });
                return;
            };
            let data = convert::data(&resume_data);
            let block = RcBlock::new(move |download: NonNull<WKDownload>| {
                // SAFETY: WebKit hands a live download.
                let download = unsafe { download.as_ref() };
                track(mtm, id, None, download);
            });
            // SAFETY: resuming on a live view with WebKit's own resume data.
            unsafe { view.resumeDownloadFromResumeData_completionHandler(&data, &block) };
        }
    }
}
