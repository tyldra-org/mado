//! The delegate every tab view has (spec 02 §4.3): navigation, UI and the
//! `bridge` message channel, plus key-value observation of the view's
//! properties. Delegates only report; decisions come back from the core
//! through [`answer`] on the main thread.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::ptr::NonNull;

use block2::RcBlock;
use mado_engine::event::{
    AuthChallenge, AuthResponse, ClientCertRequest, ClientCertResponse, DialogKind, DialogResponse,
    EngineDownloadId, EngineSecurity, FileChooserRequest, FrameInfo, GoneReason, JsDialog,
    PermissionDecision, PermissionRequest,
};
use mado_engine::page::{DropCounts, Nonce};
use mado_engine::tab::{Modifiers, PopupDecision, PopupFeatures, PopupRequest};
use mado_engine::{
    EngineEvent, EventSink, NavDecision, NavKind, NavPhase, NavRequest, NetError, PermissionKind,
    Reply, World,
};
use mado_types::TabId;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::NSEventModifierFlags;
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSKeyValueChangeKey, NSString, NSURL,
    NSURLAuthenticationChallenge, NSURLCredential, NSURLCredentialPersistence,
    NSURLSessionAuthChallengeDisposition,
};
use objc2_web_kit::{
    WKDownload, WKFrameInfo, WKMediaCaptureState, WKMediaCaptureType, WKNavigation,
    WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKNavigationType, WKOpenPanelParameters, WKPermissionDecision,
    WKScriptMessage, WKScriptMessageHandlerWithReply, WKSecurityOrigin, WKUIDelegate,
    WKUserContentController, WKWebView, WKWebViewConfiguration, WKWebpagePreferences,
    WKWindowFeatures,
};

use crate::{MainRunner, convert, timeouts};

/// What the delegates need about one tab, kept outside the adapter so a
/// delegate can read it while the adapter is busy.
#[derive(Debug)]
pub struct TabInfo {
    pub nonce: Nonce,
    /// Applies from the next navigation (spec 02 §4.4).
    pub javascript: bool,
    /// The kind of the next navigation Mado started (`Navigate`).
    pub next_kind: Option<NavKind>,
    pub last_gesture: bool,
    pub nav: u64,
    pub drops: DropCounts,
    /// The last find query and its case sensitivity, for `FindNext`.
    pub find: Option<(String, bool)>,
    /// The kind of the main-frame navigation decided last, for
    /// `NavigationStarted`.
    pub started_kind: Option<NavKind>,
}

impl TabInfo {
    pub fn new(javascript: bool) -> Self {
        Self {
            nonce: Nonce::generate().unwrap_or_else(|_| fallback_nonce()),
            javascript,
            next_kind: None,
            last_gesture: false,
            nav: 0,
            drops: DropCounts::default(),
            find: None,
            started_kind: None,
        }
    }
}

/// `getrandom` failing is not survivable for security; this nonce matches
/// nothing a page can produce, so every message is dropped.
fn fallback_nonce() -> Nonce {
    loop {
        if let Ok(n) = Nonce::generate() {
            return n;
        }
        std::thread::yield_now();
    }
}

enum Pending {
    Action {
        handler: RcBlock<dyn Fn(WKNavigationActionPolicy, NonNull<WKWebpagePreferences>)>,
        prefs: Retained<WKWebpagePreferences>,
        tab: TabId,
        /// A main-frame load of another document: it gets a fresh nonce.
        new_document: bool,
    },
    Response(RcBlock<dyn Fn(WKNavigationResponsePolicy)>),
    Permission(RcBlock<dyn Fn(WKPermissionDecision)>),
    Alert(RcBlock<dyn Fn()>),
    Confirm(RcBlock<dyn Fn(objc2::runtime::Bool)>),
    Prompt(RcBlock<dyn Fn(*mut NSString)>),
    Auth(RcBlock<dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential)>),
    Files(RcBlock<dyn Fn(*mut NSArray<NSURL>)>),
    Popup(TabId),
    Download(EngineDownloadId, RcBlock<dyn Fn(*mut NSURL)>),
}

/// A decision the core made, delivered on the main thread.
pub enum Answer {
    Nav(NavDecision),
    Permission(PermissionDecision),
    Dialog(DialogResponse),
    Auth(AuthResponse),
    ClientCert(ClientCertResponse),
    Files(Vec<PathBuf>),
    Popup(PopupDecision),
    Download(mado_engine::event::DownloadDestination),
}

thread_local! {
    static CONTEXT: RefCell<Option<(EventSink, MainRunner)>> = const { RefCell::new(None) };
    pub static INFO: RefCell<BTreeMap<TabId, TabInfo>> = const { RefCell::new(BTreeMap::new()) };
    static PENDING: RefCell<BTreeMap<u64, Pending>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

pub fn set_context(events: EventSink, runner: MainRunner) {
    CONTEXT.with_borrow_mut(|c| *c = Some((events, runner)));
}

pub(crate) fn send(event: EngineEvent) {
    let sink = CONTEXT.with_borrow(|c| c.as_ref().map(|c| c.0.clone()));
    if let Some(sink) = sink {
        sink.send(event);
    }
}

fn runner() -> Option<MainRunner> {
    CONTEXT.with_borrow(|c| c.as_ref().map(|c| c.1.clone()))
}

/// Parks `pending` and returns a reply that answers it on the main thread,
/// or with `default` after the timeout.
fn park<T: Send + 'static>(
    pending: Pending,
    default: T,
    wrap: fn(T) -> Answer,
) -> Option<Reply<T>> {
    let runner = runner()?;
    let id = NEXT.get();
    NEXT.set(id + 1);
    PENDING.with_borrow_mut(|p| p.insert(id, pending));
    let (reply, expiry) = Reply::new(default, move |value: T| {
        let answer = wrap(value);
        runner(Box::new(move || answer_on_main(id, answer)));
    });
    timeouts::arm(expiry);
    Some(reply)
}

fn answer_on_main(id: u64, answer: Answer) {
    let Some(pending) = PENDING.with_borrow_mut(|p| p.remove(&id)) else {
        return;
    };
    answer_pending(pending, answer);
}

fn answer_pending(pending: Pending, answer: Answer) {
    match (pending, answer) {
        (
            Pending::Action {
                handler,
                prefs,
                tab,
                new_document,
            },
            Answer::Nav(decision),
        ) => {
            if new_document && decision == NavDecision::Allow {
                crate::with(|a| a.rotate_nonce(tab));
            }
            let js = INFO.with_borrow(|i| i.get(&tab).is_none_or(|t| t.javascript));
            // SAFETY: setting a property on the preferences WebKit gave us.
            unsafe { prefs.setAllowsContentJavaScript(js) };
            let policy = match decision {
                NavDecision::Allow => WKNavigationActionPolicy::Allow,
                NavDecision::Download => WKNavigationActionPolicy::Download,
                NavDecision::Cancel | NavDecision::OpenExternal { .. } => {
                    WKNavigationActionPolicy::Cancel
                }
                NavDecision::Redirect { url } => {
                    crate::view::load_later(tab, url);
                    WKNavigationActionPolicy::Cancel
                }
            };
            handler.call((policy, NonNull::from(&*prefs)));
        }
        (Pending::Response(handler), Answer::Nav(decision)) => {
            let policy = match decision {
                NavDecision::Download => WKNavigationResponsePolicy::Download,
                NavDecision::Allow => WKNavigationResponsePolicy::Allow,
                _ => WKNavigationResponsePolicy::Cancel,
            };
            handler.call((policy,));
        }
        (Pending::Permission(handler), Answer::Permission(d)) => handler.call((match d {
            PermissionDecision::Grant => WKPermissionDecision::Grant,
            PermissionDecision::Deny => WKPermissionDecision::Deny,
        },)),
        (Pending::Alert(handler), Answer::Dialog(_)) => handler.call(()),
        (Pending::Confirm(handler), Answer::Dialog(d)) => {
            handler.call((objc2::runtime::Bool::new(matches!(
                d,
                DialogResponse::Accept(_)
            )),));
        }
        (Pending::Prompt(handler), Answer::Dialog(d)) => match d {
            DialogResponse::Accept(text) => {
                let s = NSString::from_str(&text.unwrap_or_default());
                handler.call((Retained::as_ptr(&s).cast_mut(),));
            }
            DialogResponse::Dismiss => handler.call((std::ptr::null_mut(),)),
        },
        (Pending::Auth(handler), Answer::Auth(a)) => match a {
            AuthResponse::Credentials { user, password } => {
                let credential = NSURLCredential::credentialWithUser_password_persistence(
                    &NSString::from_str(&user),
                    &NSString::from_str(&password),
                    NSURLCredentialPersistence::ForSession,
                );
                handler.call((
                    NSURLSessionAuthChallengeDisposition::UseCredential,
                    Retained::as_ptr(&credential).cast_mut(),
                ));
            }
            AuthResponse::Cancel => handler.call((
                NSURLSessionAuthChallengeDisposition::UseCredential,
                std::ptr::null_mut(),
            )),
        },
        (Pending::Auth(handler), Answer::ClientCert(c)) => match c {
            // Continue without a certificate.
            ClientCertResponse::Cancel => handler.call((
                NSURLSessionAuthChallengeDisposition::UseCredential,
                std::ptr::null_mut(),
            )),
            // Choosing a keychain identity arrives with the client
            // certificate sheet (M2-T07); until then WebKit decides.
            ClientCertResponse::Identity(_) => {
                handler.call((
                    NSURLSessionAuthChallengeDisposition::PerformDefaultHandling,
                    std::ptr::null_mut(),
                ));
            }
        },
        (Pending::Files(handler), Answer::Files(paths)) => {
            if paths.is_empty() {
                handler.call((std::ptr::null_mut(),));
            } else {
                let urls: Vec<Retained<NSURL>> = paths
                    .iter()
                    .map(|p| NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy())))
                    .collect();
                let list = NSArray::from_retained_slice(&urls);
                handler.call((Retained::as_ptr(&list).cast_mut(),));
            }
        }
        (Pending::Download(id, handler), Answer::Download(d)) => {
            crate::download::answer_destination(id, &handler, d);
        }
        (Pending::Popup(tab), Answer::Popup(decision)) => {
            if decision == PopupDecision::Block {
                crate::view::drop_popup(tab);
            }
        }
        (other, _) => {
            // A mismatched answer cannot happen: each reply wraps its own
            // type. Fall back to the safe default for the pending kind.
            answer_default(other);
        }
    }
}

fn answer_default(pending: Pending) {
    match pending {
        Pending::Action { .. } | Pending::Response(_) => {
            answer_pending(pending, Answer::Nav(NavDecision::Cancel))
        }
        Pending::Permission(_) => {
            answer_pending(pending, Answer::Permission(PermissionDecision::Deny))
        }
        Pending::Alert(_) | Pending::Confirm(_) | Pending::Prompt(_) => {
            answer_pending(pending, Answer::Dialog(DialogResponse::Dismiss));
        }
        Pending::Auth(_) => answer_pending(pending, Answer::Auth(AuthResponse::Cancel)),
        Pending::Files(_) => answer_pending(pending, Answer::Files(Vec::new())),
        Pending::Download(..) => answer_pending(
            pending,
            Answer::Download(mado_engine::event::DownloadDestination::Cancel),
        ),
        Pending::Popup(_) => answer_pending(pending, Answer::Popup(PopupDecision::Block)),
    }
}

fn origin_string(origin: &WKSecurityOrigin) -> String {
    // SAFETY: reading properties of a WebKit object on the main thread.
    let (scheme, host, port) = unsafe {
        (
            origin.protocol().to_string(),
            origin.host().to_string(),
            origin.port(),
        )
    };
    if port > 0 {
        format!("{scheme}://{host}:{port}")
    } else {
        format!("{scheme}://{host}")
    }
}

fn frame_info(frame: &WKFrameInfo) -> FrameInfo {
    // SAFETY: reading properties on the main thread.
    unsafe {
        FrameInfo {
            is_main_frame: frame.isMainFrame(),
            origin: origin_string(&frame.securityOrigin()),
            url: convert::url_string(frame.request().URL()),
        }
    }
}

fn modifiers(action: &WKNavigationAction) -> Modifiers {
    // SAFETY: reading properties on the main thread.
    let (flags, button) = unsafe { (action.modifierFlags(), action.buttonNumber()) };
    Modifiers {
        command: flags.contains(NSEventModifierFlags::Command),
        shift: flags.contains(NSEventModifierFlags::Shift),
        option: flags.contains(NSEventModifierFlags::Option),
        control: flags.contains(NSEventModifierFlags::Control),
        button: u8::try_from(button.max(0)).unwrap_or(0),
    }
}

fn user_gesture(action: &WKNavigationAction) -> bool {
    let sel = objc2::sel!(_isUserInitiated);
    // SAFETY: respondsToSelector: is safe on any object; _isUserInitiated
    // returns a BOOL when it exists.
    unsafe {
        if msg_send![action, respondsToSelector: sel] {
            let b: bool = msg_send![action, _isUserInitiated];
            return b;
        }
        matches!(
            action.navigationType(),
            WKNavigationType::LinkActivated | WKNavigationType::FormSubmitted
        )
    }
}

pub struct DelegateIvars {
    pub tab: TabId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoTabDelegate"]
    #[ivars = DelegateIvars]
    pub struct TabDelegate;

    unsafe impl NSObjectProtocol for TabDelegate {}

    unsafe impl WKNavigationDelegate for TabDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:preferences:decisionHandler:))]
        fn decide_action(
            &self,
            view: &WKWebView,
            action: &WKNavigationAction,
            preferences: &WKWebpagePreferences,
            handler: &block2::DynBlock<
                dyn Fn(WKNavigationActionPolicy, NonNull<WKWebpagePreferences>),
            >,
        ) {
            let tab = self.ivars().tab;
            // SAFETY: reading the action's properties on the main thread.
            let (url, method, source, main, should_download) = unsafe {
                let request = action.request();
                let source = action.sourceFrame();
                (
                    convert::url_string(request.URL()),
                    request
                        .HTTPMethod()
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| "GET".into()),
                    source
                        .map(|s| convert::url_string(s.request().URL()))
                        .unwrap_or_default(),
                    action.targetFrame().is_none_or(|f| f.isMainFrame()),
                    action.shouldPerformDownload(),
                )
            };
            let gesture = user_gesture(action);
            let kind = INFO.with_borrow_mut(|i| {
                let info = i.get_mut(&tab)?;
                if main {
                    info.last_gesture = gesture;
                }
                main.then(|| info.next_kind.take()).flatten()
            });
            let initiated_by_mado = kind.is_some();
            // SAFETY: navigationType is a plain property.
            let nav_type = unsafe { action.navigationType() };
            let kind = kind.unwrap_or(match nav_type {
                WKNavigationType::LinkActivated => NavKind::Link,
                WKNavigationType::FormSubmitted | WKNavigationType::FormResubmitted => {
                    NavKind::FormSubmit
                }
                WKNavigationType::BackForward => NavKind::BackForward,
                WKNavigationType::Reload => NavKind::Reload,
                _ => NavKind::Other,
            });
            if main {
                INFO.with_borrow_mut(|i| {
                    if let Some(info) = i.get_mut(&tab) {
                        info.started_kind = Some(kind);
                    }
                });
            }
            let request = NavRequest {
                url,
                source_url: (!initiated_by_mado && !source.is_empty()).then_some(source),
                is_main_frame: main,
                user_gesture: gesture,
                initiated_by_mado,
                kind,
                phase: NavPhase::Action {
                    modifiers: modifiers(action),
                    method,
                },
            };
            let strip = |u: &str| u.split('#').next().unwrap_or_default().to_owned();
            // SAFETY: URL of a live view on the main thread.
            let current = convert::url_string(unsafe { view.URL() });
            let pending = Pending::Action {
                handler: handler.copy(),
                prefs: preferences.retain(),
                tab,
                new_document: main && strip(&current) != strip(&request.url),
            };
            if should_download {
                answer_pending(pending, Answer::Nav(NavDecision::Download));
                return;
            }
            match park(pending, NavDecision::Cancel, Answer::Nav) {
                Some(reply) => send(EngineEvent::DecidePolicy {
                    tab,
                    request,
                    reply,
                }),
                None => tracing::error!("a navigation arrived before the adapter was ready"),
            }
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(
            &self,
            _view: &WKWebView,
            response: &WKNavigationResponse,
            handler: &block2::DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            let tab = self.ivars().tab;
            // SAFETY: reading the response's properties on the main thread.
            let (url, mime, main, can_show, attachment) = unsafe {
                let r = response.response();
                let disposition: Option<Retained<NSString>> = if msg_send![&*r, respondsToSelector: objc2::sel!(valueForHTTPHeaderField:)]
                {
                    msg_send![&*r, valueForHTTPHeaderField: &*NSString::from_str("Content-Disposition")]
                } else {
                    None
                };
                (
                    convert::url_string(r.URL()),
                    r.MIMEType().map(|m| m.to_string()).unwrap_or_default(),
                    response.isForMainFrame(),
                    response.canShowMIMEType(),
                    disposition.is_some_and(|d| {
                        d.to_string()
                            .trim_start()
                            .to_ascii_lowercase()
                            .starts_with("attachment")
                    }),
                )
            };
            let request = NavRequest {
                url,
                source_url: None,
                is_main_frame: main,
                user_gesture: false,
                initiated_by_mado: false,
                kind: NavKind::Other,
                phase: NavPhase::Response {
                    mime,
                    attachment,
                    can_show,
                },
            };
            match park(
                Pending::Response(handler.copy()),
                NavDecision::Cancel,
                Answer::Nav,
            ) {
                Some(reply) => send(EngineEvent::DecidePolicy {
                    tab,
                    request,
                    reply,
                }),
                None => handler.call((WKNavigationResponsePolicy::Cancel,)),
            }
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start(&self, view: &WKWebView, _nav: Option<&WKNavigation>) {
            let tab = self.ivars().tab;
            let (nav_id, gesture, kind) = INFO.with_borrow_mut(|i| {
                i.get_mut(&tab).map_or((0, false, NavKind::Other), |t| {
                    t.nav += 1;
                    (t.nav, t.last_gesture, t.started_kind.take().unwrap_or(NavKind::Other))
                })
            });
            // SAFETY: URL of a live view on the main thread.
            let url = convert::url_string(unsafe { view.URL() });
            send(EngineEvent::NavigationStarted {
                tab,
                url,
                nav_id,
                is_main_frame: true,
                user_gesture: gesture,
                kind,
            });
        }

        #[unsafe(method(webView:didReceiveServerRedirectForProvisionalNavigation:))]
        fn did_redirect(&self, view: &WKWebView, _nav: Option<&WKNavigation>) {
            let tab = self.ivars().tab;
            // SAFETY: URL of a live view on the main thread.
            let url = convert::url_string(unsafe { view.URL() });
            send(EngineEvent::NavigationRedirected {
                tab,
                nav_id: current_nav(tab),
                url,
            });
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, view: &WKWebView, _nav: Option<&WKNavigation>) {
            let tab = self.ivars().tab;
            // SAFETY: properties of a live view on the main thread.
            let (url, secure, trusted) = unsafe {
                (
                    convert::url_string(view.URL()),
                    view.hasOnlySecureContent(),
                    view.serverTrust().is_some(),
                )
            };
            send(EngineEvent::NavigationCommitted {
                tab,
                nav_id: current_nav(tab),
                url: url.clone(),
                is_same_document: false,
            });
            send(EngineEvent::SecurityChanged {
                tab,
                state: EngineSecurity {
                    secure: !url.starts_with("https:") || trusted,
                    mixed_content: url.starts_with("https:") && !secure,
                    certificate_subject: None,
                },
            });
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, _view: &WKWebView, _nav: Option<&WKNavigation>) {
            let tab = self.ivars().tab;
            send(EngineEvent::NavigationFinished {
                tab,
                nav_id: current_nav(tab),
            });
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(
            &self,
            view: &WKWebView,
            _nav: Option<&WKNavigation>,
            error: &NSError,
        ) {
            failed(self.ivars().tab, view, error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, view: &WKWebView, _nav: Option<&WKNavigation>, error: &NSError) {
            failed(self.ivars().tab, view, error);
        }

        #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
        fn challenge(
            &self,
            _view: &WKWebView,
            challenge: &NSURLAuthenticationChallenge,
            handler: &block2::DynBlock<
                dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential),
            >,
        ) {
            let tab = self.ivars().tab;
            let (method, host, port, realm, proxy) = {
                let space = challenge.protectionSpace();
                (
                    space.authenticationMethod().to_string(),
                    space.host().to_string(),
                    u16::try_from(space.port()).unwrap_or(0),
                    space.realm().map(|r| r.to_string()),
                    space.isProxy(),
                )
            };
            let scheme = match method.as_str() {
                "NSURLAuthenticationMethodHTTPBasic" => "basic",
                "NSURLAuthenticationMethodHTTPDigest" => "digest",
                "NSURLAuthenticationMethodNTLM" => "ntlm",
                "NSURLAuthenticationMethodNegotiate" => "negotiate",
                "NSURLAuthenticationMethodClientCertificate" => {
                    let request = ClientCertRequest { host, port };
                    match park(
                        Pending::Auth(handler.copy()),
                        ClientCertResponse::Cancel,
                        Answer::ClientCert,
                    ) {
                        Some(reply) => send(EngineEvent::ClientCertificateRequested {
                            tab,
                            request,
                            reply,
                        }),
                        None => handler.call((
                            NSURLSessionAuthChallengeDisposition::PerformDefaultHandling,
                            std::ptr::null_mut(),
                        )),
                    }
                    return;
                }
                // Server trust: WebKit evaluates it and fails the load with
                // a certificate error (M2-T08 adds the interstitial).
                _ => {
                    handler.call((
                        NSURLSessionAuthChallengeDisposition::PerformDefaultHandling,
                        std::ptr::null_mut(),
                    ));
                    return;
                }
            };
            let request = AuthChallenge {
                host,
                port,
                realm,
                scheme: scheme.into(),
                is_proxy: proxy,
            };
            match park(
                Pending::Auth(handler.copy()),
                AuthResponse::Cancel,
                Answer::Auth,
            ) {
                Some(reply) => send(EngineEvent::AuthRequested {
                    tab,
                    challenge: request,
                    reply,
                }),
                None => handler.call((
                    NSURLSessionAuthChallengeDisposition::UseCredential,
                    std::ptr::null_mut(),
                )),
            }
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, _view: &WKWebView) {
            send(EngineEvent::ProcessGone {
                tab: self.ivars().tab,
                reason: GoneReason::Crashed,
            });
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_download(
            &self,
            _view: &WKWebView,
            _action: &WKNavigationAction,
            download: &WKDownload,
        ) {
            became_download(self.ivars().tab, download);
        }

        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_download(
            &self,
            _view: &WKWebView,
            _response: &WKNavigationResponse,
            download: &WKDownload,
        ) {
            became_download(self.ivars().tab, download);
        }
    }

    unsafe impl WKUIDelegate for TabDelegate {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            _view: &WKWebView,
            configuration: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            let opener = self.ivars().tab;
            // SAFETY: reading properties on the main thread.
            let (url, f) = unsafe {
                let num =
                    |n: Option<Retained<objc2_foundation::NSNumber>>| n.map(|n| n.doubleValue());
                (
                    convert::url_string(action.request().URL()),
                    PopupFeatures {
                        x: num(features.x()),
                        y: num(features.y()),
                        width: num(features.width()),
                        height: num(features.height()),
                    },
                )
            };
            let tab = TabId::new();
            let request = PopupRequest {
                opener,
                tab,
                url,
                features: f,
                user_gesture: user_gesture(action),
                modifiers: modifiers(action),
            };
            // WebKit needs the view now, and a page sees a window object as
            // soon as one is returned, so a pop-up without a user gesture is
            // refused here, as Chrome returns null (spec 05 §7). The site's
            // "allow pop-ups" exception reaches the adapter with the address
            // bar's blocked-pop-up list (M2-T06).
            let allowed = request.user_gesture;
            allowed
                .then(|| crate::view::create_popup(opener, tab, configuration))
                .flatten()
                .and_then(|view| {
                    match park(Pending::Popup(tab), PopupDecision::Block, Answer::Popup) {
                        Some(reply) => {
                            send(EngineEvent::TabCreated { tab });
                            send(EngineEvent::NewWindowRequested {
                                tab: opener,
                                request,
                                reply,
                            });
                            Some(view)
                        }
                        None => {
                            crate::view::drop_popup(tab);
                            None
                        }
                    }
                })
        }

        #[unsafe(method(webViewDidClose:))]
        fn did_close(&self, _view: &WKWebView) {
            send(EngineEvent::CloseRequested {
                tab: self.ivars().tab,
            });
        }

        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(
            &self,
            _view: &WKWebView,
            message: &NSString,
            frame: &WKFrameInfo,
            handler: &block2::DynBlock<dyn Fn()>,
        ) {
            dialog(
                self.ivars().tab,
                DialogKind::Alert,
                message,
                frame,
                Pending::Alert(handler.copy()),
            );
        }

        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(
            &self,
            _view: &WKWebView,
            message: &NSString,
            frame: &WKFrameInfo,
            handler: &block2::DynBlock<dyn Fn(objc2::runtime::Bool)>,
        ) {
            dialog(
                self.ivars().tab,
                DialogKind::Confirm,
                message,
                frame,
                Pending::Confirm(handler.copy()),
            );
        }

        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn prompt(
            &self,
            _view: &WKWebView,
            message: &NSString,
            default: Option<&NSString>,
            frame: &WKFrameInfo,
            handler: &block2::DynBlock<dyn Fn(*mut NSString)>,
        ) {
            let kind = DialogKind::Prompt {
                default: default.map(|d| d.to_string()).unwrap_or_default(),
            };
            dialog(
                self.ivars().tab,
                kind,
                message,
                frame,
                Pending::Prompt(handler.copy()),
            );
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn capture(
            &self,
            _view: &WKWebView,
            origin: &WKSecurityOrigin,
            frame: &WKFrameInfo,
            kind: WKMediaCaptureType,
            handler: &block2::DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            let tab = self.ivars().tab;
            let info = frame_info(frame);
            let kind = if kind == WKMediaCaptureType::Microphone {
                PermissionKind::Microphone
            } else {
                PermissionKind::Camera
            };
            let request = PermissionRequest {
                kind,
                origin: origin_string(origin),
                embedder: (!info.is_main_frame).then_some(info.origin),
            };
            match park(
                Pending::Permission(handler.copy()),
                PermissionDecision::Deny,
                Answer::Permission,
            ) {
                Some(reply) => send(EngineEvent::PermissionRequested {
                    tab,
                    request,
                    reply,
                }),
                None => handler.call((WKPermissionDecision::Deny,)),
            }
        }

        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn open_panel(
            &self,
            _view: &WKWebView,
            parameters: &WKOpenPanelParameters,
            _frame: &WKFrameInfo,
            handler: &block2::DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            let tab = self.ivars().tab;
            // SAFETY: plain properties on the main thread.
            let request = unsafe {
                FileChooserRequest {
                    multiple: parameters.allowsMultipleSelection(),
                    directories: parameters.allowsDirectories(),
                    accept: Vec::new(),
                }
            };
            match park(Pending::Files(handler.copy()), Vec::new(), Answer::Files) {
                Some(reply) => send(EngineEvent::FileChooser {
                    tab,
                    request,
                    reply,
                }),
                None => handler.call((std::ptr::null_mut(),)),
            }
        }
    }

    impl TabDelegate {
        /// macOS 27's geolocation permission delegate (M0: called on
        /// 27.2), not yet in the SDK headers this crate binds.
        #[unsafe(method(webView:requestGeolocationPermissionForOrigin:initiatedByFrame:decisionHandler:))]
        fn geolocation(
            &self,
            _view: &WKWebView,
            origin: &WKSecurityOrigin,
            frame: &WKFrameInfo,
            handler: &block2::DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            let tab = self.ivars().tab;
            let info = frame_info(frame);
            let request = PermissionRequest {
                kind: PermissionKind::Location,
                origin: origin_string(origin),
                embedder: (!info.is_main_frame).then_some(info.origin),
            };
            match park(Pending::Permission(handler.copy()), PermissionDecision::Deny, Answer::Permission) {
                Some(reply) => send(EngineEvent::PermissionRequested { tab, request, reply }),
                None => handler.call((WKPermissionDecision::Deny,)),
            }
        }
    }

    unsafe impl WKScriptMessageHandlerWithReply for TabDelegate {
        #[unsafe(method(userContentController:didReceiveScriptMessage:replyHandler:))]
        fn message(
            &self,
            _controller: &WKUserContentController,
            message: &WKScriptMessage,
            reply: &block2::DynBlock<dyn Fn(*mut AnyObject, *mut NSString)>,
        ) {
            let tab = self.ivars().tab;
            // SAFETY: reading the message on the main thread.
            let (body, frame) = unsafe {
                (
                    convert::to_json(Some(&message.body())),
                    frame_info(&message.frameInfo()),
                )
            };
            let checked = INFO.with_borrow_mut(|i| {
                let info = i.get_mut(&tab)?;
                match mado_engine::page::validate(&body, &info.nonce) {
                    Ok(m) => Some(m),
                    Err(why) => {
                        info.drops.add(why);
                        None
                    }
                }
            });
            if let Some(message) = checked {
                send(EngineEvent::PageMessage {
                    tab,
                    world: World::Mado,
                    frame,
                    message,
                });
            }
            reply.call((std::ptr::null_mut(), std::ptr::null_mut()));
        }
    }
);

impl TabDelegate {
    pub fn new(mtm: MainThreadMarker, tab: TabId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars { tab });
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

fn current_nav(tab: TabId) -> u64 {
    INFO.with_borrow(|i| i.get(&tab).map_or(0, |t| t.nav))
}

fn failed(tab: TabId, view: &WKWebView, error: &NSError) {
    // SAFETY: URL of a live view on the main thread.
    let mut url = convert::url_string(unsafe { view.URL() });
    let info = error.userInfo();
    if let Some(u) = info
        .objectForKey(&NSString::from_str("NSErrorFailingURLStringKey"))
        .and_then(|v| v.downcast::<NSString>().ok())
    {
        url = u.to_string();
    }
    send(EngineEvent::NavigationFailed {
        tab,
        nav_id: current_nav(tab),
        url,
        error: NetError::from_nsurl_code(convert::error_code(error)),
    });
}

fn dialog(tab: TabId, kind: DialogKind, message: &NSString, frame: &WKFrameInfo, pending: Pending) {
    let dialog = JsDialog {
        kind,
        message: message.to_string(),
        frame: frame_info(frame),
    };
    match park(pending, DialogResponse::Dismiss, Answer::Dialog) {
        Some(reply) => send(EngineEvent::DialogRequested { tab, dialog, reply }),
        None => tracing::error!("a dialog arrived before the adapter was ready"),
    }
}

fn became_download(tab: TabId, download: &WKDownload) {
    static NEXT_DOWNLOAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = EngineDownloadId(NEXT_DOWNLOAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    if let Some(mtm) = MainThreadMarker::new() {
        crate::download::track(mtm, id, Some(tab), download);
    }
}

/// Parks a download's destination question for the core.
pub(crate) fn park_download(
    id: EngineDownloadId,
    handler: RcBlock<dyn Fn(*mut NSURL)>,
) -> Option<Reply<mado_engine::event::DownloadDestination>> {
    park(
        Pending::Download(id, handler),
        mado_engine::event::DownloadDestination::Cancel,
        Answer::Download,
    )
}

/// The key paths every tab view is observed for (spec 02 §4.3).
pub const OBSERVED: &[&str] = &[
    "title",
    "URL",
    "estimatedProgress",
    "canGoBack",
    "canGoForward",
    "hasOnlySecureContent",
    "cameraCaptureState",
    "microphoneCaptureState",
    "fullscreenState",
];

/// Private, observed only when the view responds to it: whether the page
/// is playing audio.
pub const AUDIO_KEY: &str = "_isPlayingAudio";

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoTabObserver"]
    #[ivars = DelegateIvars]
    pub struct Observer;

    unsafe impl NSObjectProtocol for Observer {}

    impl Observer {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            key: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>,
            _context: *mut std::ffi::c_void,
        ) {
            let (Some(key), Some(view)) = (key, object.and_then(|o| o.downcast_ref::<WKWebView>())) else {
                return;
            };
            observed(self.ivars().tab, &key.to_string(), view);
        }
    }
);

impl Observer {
    pub fn new(mtm: MainThreadMarker, tab: TabId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars { tab });
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

fn observed(tab: TabId, key: &str, view: &WKWebView) {
    // SAFETY: reading properties of a live view on the main thread.
    let event = unsafe {
        match key {
            "title" => EngineEvent::TitleChanged {
                tab,
                title: view.title().map(|t| t.to_string()).unwrap_or_default(),
            },
            "URL" => EngineEvent::UrlChanged {
                tab,
                url: convert::url_string(view.URL()),
            },
            "estimatedProgress" => EngineEvent::ProgressChanged {
                tab,
                progress: view.estimatedProgress(),
            },
            "canGoBack" | "canGoForward" => EngineEvent::HistoryStateChanged {
                tab,
                can_go_back: view.canGoBack(),
                can_go_forward: view.canGoForward(),
            },
            "hasOnlySecureContent" => {
                let url = convert::url_string(view.URL());
                EngineEvent::SecurityChanged {
                    tab,
                    state: EngineSecurity {
                        secure: !url.starts_with("https:") || view.serverTrust().is_some(),
                        mixed_content: url.starts_with("https:") && !view.hasOnlySecureContent(),
                        certificate_subject: None,
                    },
                }
            }
            "cameraCaptureState" | "microphoneCaptureState" => EngineEvent::CaptureChanged {
                tab,
                camera: view.cameraCaptureState() != WKMediaCaptureState::None,
                microphone: view.microphoneCaptureState() != WKMediaCaptureState::None,
                screen: false,
            },
            "fullscreenState" => EngineEvent::FullscreenChanged {
                tab,
                fullscreen: view.fullscreenState()
                    != objc2_web_kit::WKFullscreenState::NotInFullscreen,
            },
            AUDIO_KEY => {
                let playing: bool = msg_send![view, _isPlayingAudio];
                EngineEvent::AudioChanged { tab, playing }
            }
            _ => return,
        }
    };
    send(event);
}
