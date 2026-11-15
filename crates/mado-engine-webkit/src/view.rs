//! Tab views (spec 02 §4.1, §4.2, §4.4, §4.5): per-profile data store and
//! proxy, per-tab configuration, every `TabCommand`, sleeping and waking
//! through `interactionState`.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::ptr::NonNull;

use block2::RcBlock;
use mado_engine::tab::{Image, ImageFormat, MediaAction, SessionBlob};
use mado_engine::{
    Callback, ClearDataRequest, EngineError, InjectAt, PageScriptSet, PrivacyConfig, ProxyConfig,
    TabCommand, TabInitial, TabSpec, UserAgent, World,
};
use mado_types::{TabId, WindowId};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSPrintInfo, NSView, NSWindowOrderingMode,
};
use objc2_foundation::{
    NSArray, NSData, NSDate, NSDictionary, NSError, NSMutableURLRequest, NSNumber,
    NSObjectNSKeyValueObserverRegistration, NSSet, NSString, NSUUID, ns_string,
};
use objc2_web_kit::{
    WKAudiovisualMediaTypes, WKContentRuleList, WKContentRuleListStore, WKContentWorld,
    WKSnapshotConfiguration, WKUserContentController, WKUserScript, WKUserScriptInjectionTime,
    WKWebView, WKWebViewConfiguration, WKWebsiteDataRecord, WKWebsiteDataStore,
};

use crate::delegate::{self, INFO, Observer, TabDelegate, TabInfo};
use crate::scheme::SchemeHandler;
use crate::{Config, WindowHost, convert};

struct Tab {
    view: Retained<WKWebView>,
    delegate: Retained<TabDelegate>,
    observer: Retained<Observer>,
    controller: Retained<WKUserContentController>,
    window: WindowId,
    observing: Vec<&'static str>,
}

/// A tab view's place, for the extension host's cache (M3-T04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabViewInfo {
    pub window: WindowId,
    pub visible: bool,
}

/// The main-thread view of one tab, for code that needs the web view.
pub type TabView = Retained<WKWebView>;

pub struct Adapter {
    mtm: MainThreadMarker,
    store: Retained<WKWebsiteDataStore>,
    scripts: PageScriptSet,
    privacy: PrivacyConfig,
    /// The compiled third-party cookie rule (spec 08 §8), once compiled.
    cookie_rules: Option<Retained<WKContentRuleList>>,
    scheme: Retained<SchemeHandler>,
    inspectable: bool,
    tabs: BTreeMap<TabId, Tab>,
    /// A view outside any window, for resuming downloads when no tab has a
    /// view (after a relaunch, every tab may be asleep or Mado-rendered).
    resumer: Option<Retained<WKWebView>>,
}

impl std::fmt::Debug for Adapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Adapter")
            .field("tabs", &self.tabs.len())
            .finish_non_exhaustive()
    }
}

thread_local! {
    static HOST: std::cell::RefCell<Option<Box<dyn WindowHost>>> = const { std::cell::RefCell::new(None) };
}

fn content_view(window: WindowId) -> Option<Retained<NSView>> {
    HOST.with_borrow(|h| h.as_deref().and_then(|h| crate::content_view(h, window)))
}

fn mado_world(mtm: MainThreadMarker) -> Retained<WKContentWorld> {
    // SAFETY: a named world, created once and reused by WebKit.
    unsafe { WKContentWorld::worldWithName(ns_string!("mado"), mtm) }
}

fn world(mtm: MainThreadMarker, w: World) -> Retained<WKContentWorld> {
    match w {
        World::Mado => mado_world(mtm),
        // SAFETY: the page world singleton.
        World::Page => unsafe { WKContentWorld::pageWorld(mtm) },
    }
}

impl Adapter {
    pub fn new(mtm: MainThreadMarker, config: Config) -> Self {
        let store = if config.ephemeral {
            // SAFETY: a fresh in-memory store on the main thread.
            unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) }
        } else {
            let uuid = NSUUID::from_bytes(*config.profile.as_uuid().as_bytes());
            // SAFETY: one persistent store per profile UUID (macOS 14+).
            unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&uuid, mtm) }
        };
        delegate::set_context(config.events, config.runner);
        crate::menu::set_labels(config.labels);
        HOST.with_borrow_mut(|h| *h = Some(config.host));
        Self {
            mtm,
            store,
            scripts: PageScriptSet::default(),
            privacy: PrivacyConfig::default(),
            cookie_rules: None,
            scheme: SchemeHandler::new(mtm, config.resolver),
            inspectable: config.inspectable,
            tabs: BTreeMap::new(),
            resumer: None,
        }
    }

    pub fn tabs_in(&self, window: WindowId) -> Vec<TabId> {
        self.tabs
            .iter()
            .filter(|(_, t)| t.window == window)
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn cache(&self) -> BTreeMap<TabId, TabViewInfo> {
        self.tabs
            .iter()
            .map(|(id, t)| {
                (
                    *id,
                    TabViewInfo {
                        window: t.window,
                        visible: !t.view.isHidden(),
                    },
                )
            })
            .collect()
    }

    pub fn view(&self, tab: TabId) -> Option<TabView> {
        self.tabs.get(&tab).map(|t| t.view.clone())
    }

    /// The user scripts of one tab: its `bridge` with the current nonce,
    /// then the profile's scripts (spec 02 §5).
    fn install_scripts(&self, tab: TabId, controller: &WKUserContentController) {
        let nonce = INFO.with_borrow(|i| i.get(&tab).map(|t| t.nonce.clone()));
        let Some(nonce) = nonce else { return };
        let mut all = vec![(
            mado_engine::page::bridge_source(&nonce),
            World::Mado,
            InjectAt::DocumentStart,
            false,
        )];
        all.extend(
            self.scripts
                .scripts
                .iter()
                .map(|s| (s.source.clone(), s.world, s.at, s.main_frame_only)),
        );
        // SAFETY: replacing the user scripts of a controller we own.
        unsafe { controller.removeAllUserScripts() };
        for (source, w, at, main_only) in all {
            let time = match at {
                InjectAt::DocumentStart => WKUserScriptInjectionTime::AtDocumentStart,
                InjectAt::DocumentEnd => WKUserScriptInjectionTime::AtDocumentEnd,
            };
            // SAFETY: a new user script with a source string Mado bundles.
            let script = unsafe {
                WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                    WKUserScript::alloc(self.mtm),
                    &NSString::from_str(&source),
                    time,
                    main_only,
                    &world(self.mtm, w),
                )
            };
            // SAFETY: adding a script to a controller we own.
            unsafe { controller.addUserScript(&script) };
        }
    }

    /// Fills a configuration for one tab (spec 02 §4.2).
    fn configure(
        &self,
        tab: TabId,
        config: &WKWebViewConfiguration,
        delegate: &TabDelegate,
        user_agent: &UserAgent,
        javascript: bool,
    ) -> Retained<WKUserContentController> {
        // SAFETY: plain setters on a configuration not yet in use.
        unsafe {
            config.setWebsiteDataStore(&self.store);
            let controller = WKUserContentController::new(self.mtm);
            self.install_scripts(tab, &controller);
            controller.addScriptMessageHandlerWithReply_contentWorld_name(
                ProtocolObject::from_ref(delegate),
                &mado_world(self.mtm),
                ns_string!("mado"),
            );
            config.setUserContentController(&controller);
            if config
                .urlSchemeHandlerForURLScheme(ns_string!("mado"))
                .is_none()
            {
                config.setURLSchemeHandler_forURLScheme(
                    Some(ProtocolObject::from_ref(&*self.scheme)),
                    ns_string!("mado"),
                );
            }
            let prefs = config.preferences();
            prefs.setElementFullscreenEnabled(true);
            prefs.setFraudulentWebsiteWarningEnabled(self.privacy.dangerous_site_warnings);
            set_ice_filtering(&prefs, self.privacy.hide_local_ip);
            if self.privacy.block_third_party_cookies
                && let Some(rules) = &self.cookie_rules
            {
                controller.addContentRuleList(rules);
            }
            prefs.setTextInteractionEnabled(true);
            config
                .defaultWebpagePreferences()
                .setAllowsContentJavaScript(javascript);
            config.setMediaTypesRequiringUserActionForPlayback(WKAudiovisualMediaTypes::Audio);
            config.setAllowsAirPlayForMediaPlayback(true);
            config.setUpgradeKnownHostsToHTTPS(true);
            if let UserAgent::Safari { version } = user_agent {
                config.setApplicationNameForUserAgent(Some(&NSString::from_str(&format!(
                    "Version/{version} Safari/605.1.15"
                ))));
            }
            controller
        }
    }

    /// Delegates, observation and view settings for a new view.
    fn attach(
        &mut self,
        tab: TabId,
        window: WindowId,
        view: Retained<WKWebView>,
        delegate: Retained<TabDelegate>,
        controller: Retained<WKUserContentController>,
    ) {
        let observer = Observer::new(self.mtm, tab);
        let mut observing: Vec<&'static str> = delegate::OBSERVED.to_vec();
        // SAFETY: respondsToSelector: is safe on any object.
        let audio: bool =
            unsafe { msg_send![&*view, respondsToSelector: objc2::sel!(_isPlayingAudio)] };
        if audio {
            observing.push(delegate::AUDIO_KEY);
        }
        // SAFETY: the view holds its delegates weakly; `Tab` keeps them
        // alive, and observers are removed before the view goes.
        unsafe {
            view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            view.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            view.setAllowsBackForwardNavigationGestures(true);
            view.setAllowsMagnification(true);
            view.setInspectable(self.inspectable);
            for key in &observing {
                view.addObserver_forKeyPath_options_context(
                    &observer,
                    &NSString::from_str(key),
                    objc2_foundation::NSKeyValueObservingOptions::New,
                    std::ptr::null_mut(),
                );
            }
        }
        self.tabs.insert(
            tab,
            Tab {
                view,
                delegate,
                observer,
                controller,
                window,
                observing,
            },
        );
    }

    pub fn create_tab(&mut self, spec: TabSpec) -> Result<(), EngineError> {
        if self.tabs.contains_key(&spec.tab) {
            return Err(EngineError::Failed("the tab view exists".into()));
        }
        let content = content_view(spec.window).ok_or(EngineError::NoWindow)?;
        INFO.with_borrow_mut(|i| i.insert(spec.tab, TabInfo::new(spec.javascript)));
        let delegate = TabDelegate::new(self.mtm, spec.tab);
        // SAFETY: a new configuration on the main thread.
        let config = unsafe { WKWebViewConfiguration::new(self.mtm) };
        let controller = self.configure(
            spec.tab,
            &config,
            &delegate,
            &spec.user_agent,
            spec.javascript,
        );
        let frame = spec
            .frame
            .map_or_else(|| content.bounds(), |r| convert::frame_in(&content, r));
        let view = crate::menu::MadoWebView::create(self.mtm, spec.tab, frame, &config);
        // SAFETY: plain view setup on the main thread.
        unsafe {
            view.setHidden(!spec.visible);
            view.setPageZoom(spec.zoom);
        }
        insert(&content, &view);
        self.attach(spec.tab, spec.window, view.clone(), delegate, controller);
        match spec.initial {
            TabInitial::Url(url) => load(spec.tab, &view, &url, None),
            TabInitial::Restore(blob) => {
                // SAFETY: interactionState takes the NSData WebKit produced.
                unsafe { view.setInteractionState(Some(&convert::data(&blob.0))) };
                Ok(())
            }
            TabInitial::Blank | TabInitial::Popup(_) => Ok(()),
        }
    }

    pub fn destroy_tab(&mut self, tab: TabId) {
        crate::menu::forget(tab);
        crate::certificate::forget(tab);
        let Some(t) = self.tabs.remove(&tab) else {
            return;
        };
        INFO.with_borrow_mut(|i| i.remove(&tab));
        // SAFETY: tearing down a view we own on the main thread: observers
        // first, then delegates, then the view leaves its window.
        unsafe {
            for key in &t.observing {
                t.view
                    .removeObserver_forKeyPath(&t.observer, &NSString::from_str(key));
            }
            t.view.stopLoading();
            t.view.setNavigationDelegate(None);
            t.view.setUIDelegate(None);
            t.controller.removeAllScriptMessageHandlers();
        }
        t.view.removeFromSuperview();
        drop(t.delegate);
    }

    pub fn set_page_scripts(&mut self, scripts: PageScriptSet) {
        self.scripts = scripts;
        let tabs: Vec<(TabId, Retained<WKUserContentController>)> = self
            .tabs
            .iter()
            .map(|(id, t)| (*id, t.controller.clone()))
            .collect();
        // Existing pages keep theirs; the new set applies from their next
        // navigation.
        for (tab, controller) in tabs {
            self.install_scripts(tab, &controller);
        }
    }

    /// Applies the safety and privacy settings to every view (spec 08 §1,
    /// §8). The cookie rule compiles once, off the first call that needs it.
    pub fn set_privacy(&mut self, privacy: PrivacyConfig) {
        self.privacy = privacy;
        if privacy.block_third_party_cookies && self.cookie_rules.is_none() {
            compile_cookie_rules(self.mtm);
        }
        for t in self.tabs.values() {
            // SAFETY: the view's live preferences and its own controller.
            unsafe {
                t.view
                    .configuration()
                    .preferences()
                    .setFraudulentWebsiteWarningEnabled(privacy.dangerous_site_warnings);
                set_ice_filtering(&t.view.configuration().preferences(), privacy.hide_local_ip);
                if let Some(rules) = &self.cookie_rules {
                    t.controller.removeContentRuleList(rules);
                    if privacy.block_third_party_cookies {
                        t.controller.addContentRuleList(rules);
                    }
                }
            }
        }
    }

    fn cookie_rules_ready(&mut self, rules: Retained<WKContentRuleList>) {
        self.cookie_rules = Some(rules);
        self.set_privacy(self.privacy);
    }

    pub fn set_proxy(&mut self, proxy: &ProxyConfig) {
        if let Err(e) = set_proxy(&self.store, proxy) {
            tracing::error!(error = %e, "the proxy was not applied");
        }
    }

    /// A new main-frame document gets a fresh nonce (spec 02 §5.2).
    pub fn rotate_nonce(&mut self, tab: TabId) {
        let Some(controller) = self.tabs.get(&tab).map(|t| t.controller.clone()) else {
            return;
        };
        INFO.with_borrow_mut(|i| {
            if let Some(info) = i.get_mut(&tab)
                && let Ok(n) = mado_engine::page::Nonce::generate()
            {
                info.nonce = n;
            }
        });
        self.install_scripts(tab, &controller);
    }
}

/// `privacy.webrtcHideIp` (spec 08 §8): WebKit filters host ICE candidates
/// unless told otherwise through the `_iceCandidateFilteringEnabled`
/// preference, which has no public API. Without it the filter stays on.
fn set_ice_filtering(prefs: &objc2_web_kit::WKPreferences, on: bool) {
    let sel = objc2::sel!(_setICECandidateFilteringEnabled:);
    // SAFETY: a BOOL setter, called only when the preferences answer it.
    unsafe {
        let responds: bool = msg_send![prefs, respondsToSelector: sel];
        if responds {
            let _: () = msg_send![prefs, _setICECandidateFilteringEnabled: on];
        } else if !on {
            tracing::warn!("WebKit has no ICE candidate filtering switch; local IPs stay hidden");
        }
    }
}

/// Third-party requests carry no cookies (WebKit content rule
/// `block-cookies`, spec 08 §8).
const COOKIE_RULES: &str = r#"[{"trigger":{"url-filter":".*","load-type":["third-party"]},"action":{"type":"block-cookies"}}]"#;

fn compile_cookie_rules(mtm: MainThreadMarker) {
    // SAFETY: the app's rule list store, on the main thread.
    let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(mtm) }) else {
        tracing::error!("WebKit has no content rule list store");
        return;
    };
    let done = RcBlock::new(|list: *mut WKContentRuleList, error: *mut NSError| {
        // SAFETY: WebKit hands a list or an error, retained for the call.
        match unsafe { Retained::retain(list) } {
            Some(list) => {
                crate::with(|a| a.cookie_rules_ready(list));
            }
            None => {
                // SAFETY: as above.
                let error = unsafe { Retained::retain(error) }
                    .map(|e| e.localizedDescription().to_string())
                    .unwrap_or_default();
                tracing::error!(%error, "the third-party cookie rule did not compile");
            }
        }
    });
    // SAFETY: compiling a constant rule list with a completion block.
    unsafe {
        store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
            Some(ns_string!("mado.thirdPartyCookies")),
            Some(&NSString::from_str(COOKIE_RULES)),
            Some(&done),
        );
    }
}

fn insert(content: &NSView, view: &WKWebView) {
    match content.subviews().firstObject() {
        Some(ui) => {
            content.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Above, Some(&ui))
        }
        None => content.addSubview(view),
    }
}

fn load(
    tab: TabId,
    view: &WKWebView,
    url: &str,
    referrer: Option<&str>,
) -> Result<(), EngineError> {
    let ns_url = convert::ns_url(url).ok_or_else(|| EngineError::Failed("not a URL".into()))?;
    let request = NSMutableURLRequest::requestWithURL(&ns_url);
    if let Some(r) = referrer {
        request.setValue_forHTTPHeaderField(Some(&NSString::from_str(r)), ns_string!("Referer"));
    }
    INFO.with_borrow_mut(|i| {
        if let Some(info) = i.get_mut(&tab)
            && info.next_kind.is_none()
        {
            info.next_kind = Some(mado_engine::NavKind::Typed);
        }
    });
    // SAFETY: loading a request in a live view on the main thread.
    unsafe { view.loadRequest(&request) };
    Ok(())
}

/// HTTPS-first's redirect: load `url` after the cancelled navigation.
pub fn load_later(tab: TabId, url: String) {
    let view = crate::with(|a| a.view(tab)).flatten();
    if let Some(view) = view {
        INFO.with_borrow_mut(|i| {
            if let Some(info) = i.get_mut(&tab) {
                info.next_kind = Some(mado_engine::NavKind::Redirect);
            }
        });
        let _ = load(tab, &view, &url, None);
    }
}

/// The view WebKit needs right away for `window.open`; it waits, hidden
/// in the opener's window, for the core's decision.
pub fn create_popup(
    opener: TabId,
    tab: TabId,
    config: &WKWebViewConfiguration,
) -> Option<Retained<WKWebView>> {
    crate::with(|a| {
        let window = a.tabs.get(&opener)?.window;
        let content = content_view(window)?;
        let javascript = INFO.with_borrow(|i| i.get(&opener).is_none_or(|t| t.javascript));
        INFO.with_borrow_mut(|i| i.insert(tab, TabInfo::new(javascript)));
        let delegate = TabDelegate::new(a.mtm, tab);
        let controller = a.configure(
            tab,
            config,
            &delegate,
            &UserAgent::EngineDefault,
            javascript,
        );
        // SAFETY: a new view with WebKit's configuration, as it requires.
        let view = crate::menu::MadoWebView::create(a.mtm, tab, content.bounds(), config);
        view.setHidden(true);
        insert(&content, &view);
        a.attach(tab, window, view.clone(), delegate, controller);
        Some(view)
    })
    .flatten()
}

pub fn drop_popup(tab: TabId) {
    crate::with(|a| a.destroy_tab(tab));
}

/// Clicks a point of a tab's page, given in CSS pixels from its top left,
/// as a person would: AppKit mouse events with `modifiers`, delivered to
/// the view under the point. For end-to-end tests (spec 16 §3); never moves
/// the pointer.
pub fn synthesize_click(
    tab: TabId,
    x: f64,
    y: f64,
    modifiers: mado_engine::tab::Modifiers,
) -> Result<(), EngineError> {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
    let view = crate::with(|a| a.tabs.get(&tab).map(|t| t.view.clone()))
        .flatten()
        .ok_or_else(|| EngineError::Failed("no view for that tab".into()))?;
    let window = view
        .window()
        .ok_or_else(|| EngineError::Failed("the tab is not in a window".into()))?;
    let bounds = view.bounds();
    let local = objc2_foundation::NSPoint::new(
        x,
        if view.isFlipped() {
            y
        } else {
            bounds.size.height - y
        },
    );
    let in_window = view.convertPoint_toView(local, None);
    // SAFETY: walking a live view tree on the main thread.
    let superview = unsafe { view.superview() };
    let target = view
        .hitTest(view.convertPoint_toView(local, superview.as_deref()))
        .ok_or_else(|| EngineError::Failed("nothing at that point".into()))?;
    let mut flags = NSEventModifierFlags::empty();
    for (on, flag) in [
        (modifiers.command, NSEventModifierFlags::Command),
        (modifiers.shift, NSEventModifierFlags::Shift),
        (modifiers.option, NSEventModifierFlags::Option),
        (modifiers.control, NSEventModifierFlags::Control),
    ] {
        if on {
            flags |= flag;
        }
    }
    let time = objc2_foundation::NSProcessInfo::processInfo().systemUptime();
    for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            in_window,
            flags,
            time,
            window.windowNumber(),
            None,
            0,
            1,
            1.0,
        )
        .ok_or_else(|| EngineError::Failed("AppKit refused the mouse event".into()))?;
        if kind == NSEventType::LeftMouseDown {
            target.mouseDown(&event);
        } else {
            target.mouseUp(&event);
        }
    }
    Ok(())
}

/// A view on the profile's data store, for resuming a download (WebKit
/// resumes through a web view): a tab's, or one kept outside any window.
pub fn resume_view() -> Option<Retained<WKWebView>> {
    crate::with(|a| {
        if let Some(t) = a.tabs.values().next() {
            return t.view.clone();
        }
        a.resumer
            .get_or_insert_with(|| {
                // SAFETY: a plain configuration on the main thread, with the
                // profile's store (and so its proxy and cookies).
                unsafe {
                    let config = WKWebViewConfiguration::new(a.mtm);
                    config.setWebsiteDataStore(&a.store);
                    WKWebView::initWithFrame_configuration(
                        WKWebView::alloc(a.mtm),
                        objc2_foundation::NSRect::ZERO,
                        &config,
                    )
                }
            })
            .clone()
    })
}

fn failed(e: Option<&NSError>) -> EngineError {
    EngineError::Failed(
        e.map(|e| e.localizedDescription().to_string())
            .unwrap_or_else(|| "unknown".into()),
    )
}

/// Runs one `TabCommand` (spec 02 §4.4). Commands whose result comes later
/// answer their callback from WebKit's completion handler.
pub fn apply(tab: TabId, cmd: TabCommand) -> Result<(), EngineError> {
    let Some((view, mtm, window)) =
        crate::with(|a| a.tabs.get(&tab).map(|t| (t.view.clone(), a.mtm, t.window))).flatten()
    else {
        cmd.fail(EngineError::NoTab);
        return Err(EngineError::NoTab);
    };
    // SAFETY: calls on a live view we own, on the main thread; completion
    // blocks own what they need.
    unsafe {
        match cmd {
            TabCommand::Navigate {
                url,
                referrer,
                kind,
            } => {
                INFO.with_borrow_mut(|i| {
                    if let Some(info) = i.get_mut(&tab) {
                        info.next_kind = Some(kind);
                    }
                });
                load(tab, &view, &url, referrer.as_deref())?;
            }
            TabCommand::Reload { bypass_cache } => {
                let _ = if bypass_cache {
                    view.reloadFromOrigin()
                } else {
                    view.reload()
                };
            }
            TabCommand::Stop => view.stopLoading(),
            TabCommand::GoBack => {
                let _ = view.goBack();
            }
            TabCommand::GoForward => {
                let _ = view.goForward();
            }
            TabCommand::GoToIndex(offset) => {
                let item = view
                    .backForwardList()
                    .itemAtIndex(offset as isize)
                    .ok_or_else(|| EngineError::Failed("no such history entry".into()))?;
                let _ = view.goToBackForwardListItem(&item);
            }
            TabCommand::SetFrame(rect) => {
                if let Some(content) = content_view(window) {
                    view.setFrame(convert::frame_in(&content, rect));
                }
            }
            TabCommand::SetWindow(to) => {
                let content = content_view(to).ok_or(EngineError::NoWindow)?;
                view.removeFromSuperview();
                insert(&content, &view);
                crate::with(|a| {
                    if let Some(t) = a.tabs.get_mut(&tab) {
                        t.window = to;
                    }
                });
            }
            TabCommand::SetVisible(v) => view.setHidden(!v),
            TabCommand::Focus => {
                if let Some(w) = view.window() {
                    w.makeFirstResponder(Some(&view));
                }
            }
            TabCommand::SetZoom(z) => view.setPageZoom(z.clamp(0.25, 5.0)),
            TabCommand::SetMuted(muted) => post(
                &view,
                mtm,
                World::Mado,
                &serde_json::json!({ "type": "mute", "muted": muted }),
            ),
            TabCommand::SetMediaSuspended(s) => {
                view.setAllMediaPlaybackSuspended_completionHandler(s, None)
            }
            TabCommand::Find(req) => {
                INFO.with_borrow_mut(|i| {
                    if let Some(info) = i.get_mut(&tab) {
                        info.find = Some((req.query.clone(), req.case_sensitive));
                    }
                });
                highlight(&view, mtm, &req.query, req.case_sensitive, !req.forward);
                post(
                    &view,
                    mtm,
                    World::Mado,
                    &serde_json::json!({ "type": "find", "query": req.query, "caseSensitive": req.case_sensitive }),
                );
            }
            TabCommand::FindNext { forward } => {
                if let Some((query, case)) =
                    INFO.with_borrow(|i| i.get(&tab).and_then(|t| t.find.clone()))
                {
                    highlight(&view, mtm, &query, case, !forward);
                }
                post(
                    &view,
                    mtm,
                    World::Mado,
                    &serde_json::json!({ "type": "findNext", "forward": forward }),
                );
            }
            TabCommand::FindClear => {
                INFO.with_borrow_mut(|i| {
                    if let Some(info) = i.get_mut(&tab) {
                        info.find = None;
                    }
                });
                // The find script also clears the selection WebKit made.
                post(
                    &view,
                    mtm,
                    World::Mado,
                    &serde_json::json!({ "type": "findClear" }),
                );
            }
            TabCommand::Print => {
                let op = view.printOperationWithPrintInfo(&NSPrintInfo::sharedPrintInfo());
                op.setShowsPrintPanel(true);
                match view.window() {
                    Some(w) => op.runOperationModalForWindow_delegate_didRunSelector_contextInfo(
                        &w,
                        None,
                        None,
                        std::ptr::null_mut(),
                    ),
                    None => {
                        op.runOperation();
                    }
                }
            }
            TabCommand::SavePdf { path } => {
                let block = RcBlock::new(move |data: *mut NSData, error: *mut NSError| match data
                    .as_ref()
                {
                    Some(d) => {
                        if let Err(e) = convert::write_file(&path, d) {
                            tracing::error!(error = %e, "the PDF was not written");
                        }
                    }
                    None => {
                        tracing::error!(error = %failed(error.as_ref()), "the PDF was not made")
                    }
                });
                view.createPDFWithConfiguration_completionHandler(None, &block);
            }
            TabCommand::SaveArchive { path } => {
                let block = RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
                    if let Some(d) = data.as_ref() {
                        if let Err(e) = convert::write_file(&path, d) {
                            tracing::error!(error = %e, "the web archive was not written");
                        }
                    } else {
                        tracing::error!(error = %failed(error.as_ref()), "the web archive was not made");
                    }
                });
                view.createWebArchiveDataWithCompletionHandler(&block);
            }
            TabCommand::Snapshot {
                full_page: _,
                reply,
            } => {
                let config = WKSnapshotConfiguration::new(mtm);
                let width = view.bounds().size.width;
                if width > 1600.0 {
                    config.setSnapshotWidth(Some(&NSNumber::new_f64(1600.0)));
                }
                let reply = std::cell::Cell::new(Some(reply));
                let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                    let Some(reply) = reply.take() else { return };
                    reply(match image.as_ref().and_then(jpeg) {
                        Some(img) => Ok(img),
                        None => Err(failed(error.as_ref())),
                    });
                });
                view.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
            }
            TabCommand::EvaluateInWorld {
                world: w,
                source,
                reply,
            } => {
                let reply = std::cell::Cell::new(Some(reply));
                let block = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
                    let Some(reply) = reply.take() else { return };
                    reply(if error.is_null() {
                        Ok(convert::to_json(value.as_ref()))
                    } else {
                        Err(failed(error.as_ref()))
                    });
                });
                view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                    &NSString::from_str(&source),
                    None,
                    None,
                    &world(mtm, w),
                    Some(&block),
                );
            }
            TabCommand::PostToPage { world: w, message } => post(&view, mtm, w, &message),
            TabCommand::SaveSession { reply } => {
                let state = view.interactionState();
                let bytes = state
                    .as_deref()
                    .and_then(|s| s.downcast_ref::<NSData>())
                    .map(convert::bytes);
                reply(
                    bytes
                        .map(SessionBlob)
                        .ok_or_else(|| EngineError::Failed("no interaction state".into())),
                );
            }
            TabCommand::ShowDevTools => show_inspector(&view)?,
            TabCommand::ShowCertificate => crate::certificate::show(tab, &view)?,
            TabCommand::StopCapture(kind) => {
                let none = objc2_web_kit::WKMediaCaptureState::None;
                match kind {
                    mado_engine::PermissionKind::Camera => {
                        view.setCameraCaptureState_completionHandler(none, None);
                    }
                    mado_engine::PermissionKind::Microphone => {
                        view.setMicrophoneCaptureState_completionHandler(none, None);
                    }
                    _ => return Err(EngineError::Unsupported("only camera and microphone stop")),
                }
            }
            TabCommand::ShowErrorPage { html, url } => {
                // A simulated response keeps `url` as the page's URL, where
                // loadHTMLString would show about:blank.
                if let Some(u) = objc2_foundation::NSURL::URLWithString(&NSString::from_str(&url)) {
                    let request = objc2_foundation::NSURLRequest::requestWithURL(&u);
                    view.loadSimulatedRequest_responseHTMLString(
                        &request,
                        &NSString::from_str(&html),
                    );
                }
            }
            TabCommand::ShowSource => {
                return Err(EngineError::Unsupported(
                    "view source arrives with internal pages (M2-T14)",
                ));
            }
            TabCommand::SetReader(_) => {
                return Err(EngineError::Unsupported("reader mode arrives in M2-T08"));
            }
            TabCommand::SetJavaScriptEnabled(js) => INFO.with_borrow_mut(|i| {
                if let Some(info) = i.get_mut(&tab) {
                    info.javascript = js;
                }
            }),
            TabCommand::SetImagesEnabled(_) => {
                return Err(EngineError::Unsupported(
                    "per-site images arrive with site settings (M2-T07)",
                ));
            }
            TabCommand::ExitFullscreen => {
                view.closeAllMediaPresentationsWithCompletionHandler(None)
            }
            TabCommand::MediaControl(action) => match action {
                MediaAction::Pause => view.pauseAllMediaPlaybackWithCompletionHandler(None),
                MediaAction::Play => {
                    view.setAllMediaPlaybackSuspended_completionHandler(false, None)
                }
                MediaAction::PlayPause | MediaAction::Next | MediaAction::Previous => {
                    let name = match action {
                        MediaAction::Next => "next",
                        MediaAction::Previous => "previous",
                        _ => "playPause",
                    };
                    post(
                        &view,
                        mtm,
                        World::Mado,
                        &serde_json::json!({ "type": "media", "action": name }),
                    );
                }
            },
        }
    }
    Ok(())
}

/// WebKit's own find highlight, which selects the next match.
fn highlight(
    view: &WKWebView,
    mtm: MainThreadMarker,
    query: &str,
    case_sensitive: bool,
    backwards: bool,
) {
    // SAFETY: a find configuration and a find on a live view, main thread.
    unsafe {
        let config = objc2_web_kit::WKFindConfiguration::new(mtm);
        config.setCaseSensitive(case_sensitive);
        config.setBackwards(backwards);
        config.setWraps(true);
        let block = RcBlock::new(|_r: NonNull<objc2_web_kit::WKFindResult>| {});
        view.findString_withConfiguration_completionHandler(
            &NSString::from_str(query),
            Some(&config),
            &block,
        );
    }
}

/// A message to the page scripts' inbox (`Symbol.for("mado.inbox")` in
/// the world), which the page script tasks fill (M2-T04).
fn post(view: &WKWebView, mtm: MainThreadMarker, w: World, message: &serde_json::Value) {
    let Some(arg) = convert::from_json(message) else {
        return;
    };
    let args = NSDictionary::from_retained_objects(&[ns_string!("message")], &[arg]);
    // SAFETY: calling a function in the page with a JSON argument.
    unsafe {
        view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
            ns_string!("globalThis[Symbol.for(\"mado.inbox\")]?.(message)"),
            Some(&args),
            None,
            &world(mtm, w),
            None,
        );
    }
}

fn jpeg(image: &NSImage) -> Option<Image> {
    let tiff = image.TIFFRepresentation()?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    let quality: Retained<AnyObject> = NSNumber::new_f64(0.7).into_super().into_super().into();
    let props: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_retained_objects(
        // SAFETY: the AppKit constant key for JPEG quality.
        &[unsafe { objc2_app_kit::NSImageCompressionFactor }],
        &[quality],
    );
    // SAFETY: encoding a bitmap we hold.
    let data =
        unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::JPEG, &props) }?;
    Some(Image {
        width: u32::try_from(rep.pixelsWide()).unwrap_or(0),
        height: u32::try_from(rep.pixelsHigh()).unwrap_or(0),
        format: ImageFormat::Jpeg,
        bytes: convert::bytes(&data),
    })
}

/// Web Inspector through the private `_inspector` (M0): checked with
/// `respondsToSelector:` first.
fn show_inspector(view: &WKWebView) -> Result<(), EngineError> {
    // SAFETY: KVC on WebKit's preferences and private selectors, each
    // checked with respondsToSelector: before it is sent.
    unsafe {
        let config = view.configuration();
        let prefs = config.preferences();
        let yes = NSNumber::new_bool(true);
        let () = msg_send![&*prefs, setValue: &*yes, forKey: ns_string!("developerExtrasEnabled")];
        let has: bool = msg_send![view, respondsToSelector: objc2::sel!(_inspector)];
        if !has {
            return Err(EngineError::Unsupported(
                "Web Inspector (use the page's context menu)",
            ));
        }
        let inspector: Option<Retained<AnyObject>> = msg_send![view, _inspector];
        let inspector = inspector.ok_or(EngineError::Unsupported("Web Inspector"))?;
        let can_show: bool = msg_send![&*inspector, respondsToSelector: objc2::sel!(show)];
        if !can_show {
            return Err(EngineError::Unsupported("Web Inspector"));
        }
        let () = msg_send![&*inspector, show];
    }
    Ok(())
}

pub fn clear_data(req: ClearDataRequest, done: Callback<Result<(), EngineError>>) {
    let Some((store, mtm)) = crate::with(|a| (a.store.clone(), a.mtm)) else {
        done(Err(EngineError::Unsupported("WebKit is not installed")));
        return;
    };
    let k = req.kinds;
    // SAFETY: WebKit's data type constants.
    let names: Vec<&NSString> = unsafe {
        let mut v = Vec::new();
        if k.cookies {
            v.push(objc2_web_kit::WKWebsiteDataTypeCookies);
        }
        if k.cache {
            v.extend([
                objc2_web_kit::WKWebsiteDataTypeDiskCache,
                objc2_web_kit::WKWebsiteDataTypeMemoryCache,
                objc2_web_kit::WKWebsiteDataTypeFetchCache,
            ]);
        }
        if k.local_storage {
            v.extend([
                objc2_web_kit::WKWebsiteDataTypeLocalStorage,
                objc2_web_kit::WKWebsiteDataTypeSessionStorage,
            ]);
        }
        if k.indexed_db {
            v.push(objc2_web_kit::WKWebsiteDataTypeIndexedDBDatabases);
        }
        if k.service_workers {
            v.push(objc2_web_kit::WKWebsiteDataTypeServiceWorkerRegistrations);
        }
        if k.other_site_data {
            v.extend([
                objc2_web_kit::WKWebsiteDataTypeWebSQLDatabases,
                objc2_web_kit::WKWebsiteDataTypeFileSystem,
                objc2_web_kit::WKWebsiteDataTypeMediaKeys,
                objc2_web_kit::WKWebsiteDataTypeOfflineWebApplicationCache,
            ]);
        }
        v
    };
    let _ = mtm;
    let types = NSSet::from_slice(&names);
    let done = std::cell::Cell::new(Some(done));
    match req.hosts {
        None => {
            #[allow(clippy::cast_precision_loss)]
            let since = req.since.map_or(0.0, |ms| ms as f64 / 1000.0);
            let date = NSDate::dateWithTimeIntervalSince1970(since);
            let block = RcBlock::new(move || {
                if let Some(d) = done.take() {
                    d(Ok(()));
                }
            });
            // SAFETY: removing data from our store with a completion block.
            unsafe {
                store.removeDataOfTypes_modifiedSince_completionHandler(&types, &date, &block)
            };
        }
        Some(hosts) => {
            let store2 = store.clone();
            let types2 = types.clone();
            let block = RcBlock::new(move |records: NonNull<NSArray<WKWebsiteDataRecord>>| {
                // SAFETY: WebKit hands a valid array.
                let records = unsafe { records.as_ref() };
                let chosen: Vec<Retained<WKWebsiteDataRecord>> = records
                    .iter()
                    .filter(|r| {
                        // SAFETY: a record's display name is a registrable domain.
                        let name = unsafe { r.displayName() }.to_string();
                        hosts
                            .iter()
                            .any(|h| h == &name || h.ends_with(&format!(".{name}")))
                    })
                    .collect();
                let list = NSArray::from_retained_slice(&chosen);
                let finish = std::cell::Cell::new(done.take());
                let after = RcBlock::new(move || {
                    if let Some(d) = finish.take() {
                        d(Ok(()));
                    }
                });
                // SAFETY: removing the chosen records from our store.
                unsafe {
                    store2
                        .removeDataOfTypes_forDataRecords_completionHandler(&types2, &list, &after)
                };
            });
            // SAFETY: fetching records from our store with a completion block.
            unsafe { store.fetchDataRecordsOfTypes_completionHandler(&types, &block) };
        }
    }
}

#[link(name = "Network", kind = "framework")]
unsafe extern "C" {
    fn nw_endpoint_create_host(
        hostname: *const std::ffi::c_char,
        port: *const std::ffi::c_char,
    ) -> *mut AnyObject;
    fn nw_proxy_config_create_http_connect(
        endpoint: *mut AnyObject,
        tls: *mut AnyObject,
    ) -> *mut AnyObject;
    fn nw_proxy_config_create_socksv5(endpoint: *mut AnyObject) -> *mut AnyObject;
    fn nw_proxy_config_add_excluded_domain(config: *mut AnyObject, domain: *const std::ffi::c_char);
}

/// `host:port` to an `nw_endpoint_t`.
fn endpoint(address: &str) -> Result<Retained<AnyObject>, String> {
    let (host, port) = address
        .rsplit_once(':')
        .ok_or_else(|| format!("{address} has no port"))?;
    port.parse::<u16>()
        .map_err(|_| format!("{address} has a bad port"))?;
    let host = CString::new(host.trim_matches(['[', ']'])).map_err(|e| e.to_string())?;
    let port = CString::new(port).map_err(|e| e.to_string())?;
    // SAFETY: valid C strings; the function returns a +1 object.
    unsafe { Retained::from_raw(nw_endpoint_create_host(host.as_ptr(), port.as_ptr())) }
        .ok_or_else(|| "nw_endpoint_create_host failed".into())
}

/// `WKWebsiteDataStore.proxyConfigurations` (macOS 14+): empty for the
/// system proxy, the given servers for manual.
fn set_proxy(store: &WKWebsiteDataStore, proxy: &ProxyConfig) -> Result<(), String> {
    let mut configs: Vec<Retained<AnyObject>> = Vec::new();
    if let ProxyConfig::Manual {
        http,
        socks5,
        bypass,
    } = proxy
    {
        // SAFETY: each create function returns a +1 object or null.
        unsafe {
            if !http.is_empty() {
                let e = endpoint(http)?;
                let c = Retained::from_raw(nw_proxy_config_create_http_connect(
                    Retained::as_ptr(&e).cast_mut(),
                    std::ptr::null_mut(),
                ))
                .ok_or("nw_proxy_config_create_http_connect failed")?;
                configs.push(c);
            }
            if !socks5.is_empty() {
                let e = endpoint(socks5)?;
                let c = Retained::from_raw(nw_proxy_config_create_socksv5(
                    Retained::as_ptr(&e).cast_mut(),
                ))
                .ok_or("nw_proxy_config_create_socksv5 failed")?;
                configs.push(c);
            }
            for c in &configs {
                for domain in bypass {
                    if let Ok(d) = CString::new(domain.as_str()) {
                        nw_proxy_config_add_excluded_domain(
                            Retained::as_ptr(c).cast_mut(),
                            d.as_ptr(),
                        );
                    }
                }
            }
        }
    }
    // "Direct" has no Network.framework form; it differs from the system
    // proxy only when one is configured, and is treated as the system's.
    let list = NSArray::from_retained_slice(&configs);
    // SAFETY: respondsToSelector: is safe; the setter takes an NSArray of
    // nw_proxy_config_t (macOS 14+).
    unsafe {
        let responds: bool =
            msg_send![store, respondsToSelector: objc2::sel!(setProxyConfigurations:)];
        if !responds {
            return Err("this macOS has no proxyConfigurations".into());
        }
        let () = msg_send![store, setProxyConfigurations: &*list];
    }
    Ok(())
}
