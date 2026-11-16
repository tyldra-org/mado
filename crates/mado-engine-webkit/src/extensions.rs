//! The extension host (spec 09 §5.1): one `WKWebExtensionController` per
//! profile, contexts carrying the Chrome ID, and Mado's windows and tabs as
//! `WKWebExtensionWindow` and `WKWebExtensionTab` objects that read the
//! main-thread cache the core keeps in sync.
//!
//! Selectors used (macOS 15.4 SDK, `WKWebExtension*.h`):
//! `+[WKWebExtension extensionWithResourceBaseURL:completionHandler:]`,
//! `+[WKWebExtensionContext contextForExtension:]`,
//! `-[WKWebExtensionContext setUniqueIdentifier:]`,
//! `setPermissionStatus:forPermission:`, `setPermissionStatus:forMatchPattern:`,
//! `-[WKWebExtensionController loadExtensionContext:error:]`,
//! `unloadExtensionContext:error:`, `didOpenWindow:`, `didCloseWindow:`,
//! `didFocusWindow:`, `didOpenTab:`, `didCloseTab:windowIsClosing:`,
//! `didActivateTab:previousActiveTab:`, `didChangeTabProperties:forTab:`;
//! the delegate's `webExtensionController:openWindowsForExtensionContext:`,
//! `…focusedWindowForExtensionContext:`,
//! `…openNewTabUsingConfiguration:forExtensionContext:completionHandler:`,
//! `…openNewWindowUsingConfiguration:forExtensionContext:completionHandler:`,
//! `…openOptionsPageForExtensionContext:completionHandler:`; the tab's
//! `windowForWebExtensionContext:`, `indexInWindowForWebExtensionContext:`,
//! `webViewForWebExtensionContext:`, `titleForWebExtensionContext:`,
//! `urlForWebExtensionContext:`, `isSelectedForWebExtensionContext:`,
//! `isPinnedForWebExtensionContext:`; the window's
//! `tabsForWebExtensionContext:`, `activeTabForWebExtensionContext:`,
//! `windowTypeForWebExtensionContext:`, `windowStateForWebExtensionContext:`,
//! `isPrivateForWebExtensionContext:`, `frameForWebExtensionContext:`.

use std::cell::RefCell;
use std::collections::BTreeMap;

use block2::RcBlock;
use mado_engine::{
    Callback, EngineError, EngineEvent, ExtensionLoad, ExtensionTab, ExtensionWindow,
    ExtensionWindows,
};
use mado_types::{ProfileId, TabId, WindowId};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSArray, NSError, NSString, NSUInteger, NSURL, NSUUID};
use objc2_web_kit::{
    WKWebExtension, WKWebExtensionContext, WKWebExtensionContextPermissionStatus,
    WKWebExtensionController, WKWebExtensionControllerConfiguration,
    WKWebExtensionControllerDelegate, WKWebExtensionMatchPattern, WKWebExtensionTab,
    WKWebExtensionTabChangedProperties, WKWebExtensionTabConfiguration, WKWebExtensionWindow,
    WKWebExtensionWindowConfiguration, WKWebExtensionWindowState, WKWebExtensionWindowType,
    WKWebView, WKWebsiteDataStore,
};

struct Host {
    controller: Retained<WKWebExtensionController>,
    _delegate: Retained<ControllerDelegate>,
    contexts: BTreeMap<String, Retained<WKWebExtensionContext>>,
    cache: ExtensionWindows,
    windows: BTreeMap<WindowId, Retained<MadoExtensionWindow>>,
    tabs: BTreeMap<TabId, Retained<MadoExtensionTab>>,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Host) -> R) -> Option<R> {
    HOST.with(|h| h.try_borrow_mut().ok()?.as_mut().map(f))
}

/// The cache entry of a window or tab.
fn window_of(id: WindowId) -> Option<ExtensionWindow> {
    HOST.with(|h| {
        h.try_borrow()
            .ok()?
            .as_ref()?
            .cache
            .windows
            .iter()
            .find(|w| w.id == id)
            .cloned()
    })
}

fn tab_of(id: TabId) -> Option<(ExtensionWindow, usize, ExtensionTab)> {
    HOST.with(|h| {
        let h = h.try_borrow().ok()?;
        h.as_ref()?.cache.windows.iter().find_map(|w| {
            let i = w.tabs.iter().position(|t| t.id == id)?;
            Some((w.clone(), i, w.tabs[i].clone()))
        })
    })
}

fn window_obj(id: WindowId) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
    with(|h| h.windows.get(&id).cloned())
        .flatten()
        .map(ProtocolObject::from_retained)
}

fn tab_obj(id: TabId) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
    with(|h| h.tabs.get(&id).cloned())
        .flatten()
        .map(ProtocolObject::from_retained)
}

#[derive(Debug)]
pub struct WindowIvars {
    id: WindowId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoExtensionWindow"]
    #[ivars = WindowIvars]
    pub struct MadoExtensionWindow;

    unsafe impl NSObjectProtocol for MadoExtensionWindow {}

    unsafe impl WKWebExtensionWindow for MadoExtensionWindow {
        #[unsafe(method_id(tabsForWebExtensionContext:))]
        fn tabs(
            &self,
            _context: &WKWebExtensionContext,
        ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionTab>>> {
            let tabs: Vec<Retained<ProtocolObject<dyn WKWebExtensionTab>>> =
                window_of(self.ivars().id)
                    .map(|w| w.tabs.iter().filter_map(|t| tab_obj(t.id)).collect())
                    .unwrap_or_default();
            NSArray::from_retained_slice(&tabs)
        }

        #[unsafe(method_id(activeTabForWebExtensionContext:))]
        fn active_tab(
            &self,
            _context: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
            window_of(self.ivars().id)
                .and_then(|w| w.active)
                .and_then(tab_obj)
        }

        #[unsafe(method(windowTypeForWebExtensionContext:))]
        fn window_type(&self, _context: &WKWebExtensionContext) -> WKWebExtensionWindowType {
            WKWebExtensionWindowType::Normal
        }

        #[unsafe(method(windowStateForWebExtensionContext:))]
        fn window_state(&self, _context: &WKWebExtensionContext) -> WKWebExtensionWindowState {
            WKWebExtensionWindowState::Normal
        }

        #[unsafe(method(isPrivateForWebExtensionContext:))]
        fn is_private(&self, _context: &WKWebExtensionContext) -> bool {
            false
        }

        #[unsafe(method(frameForWebExtensionContext:))]
        fn frame(&self, _context: &WKWebExtensionContext) -> objc2_foundation::NSRect {
            HOST_WINDOWS
                .with_borrow(|f| f(self.ivars().id))
                .unwrap_or(objc2_foundation::NSRect::ZERO)
        }
    }
);

#[derive(Debug)]
pub struct TabIvars {
    id: TabId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoExtensionTab"]
    #[ivars = TabIvars]
    pub struct MadoExtensionTab;

    unsafe impl NSObjectProtocol for MadoExtensionTab {}

    unsafe impl WKWebExtensionTab for MadoExtensionTab {
        #[unsafe(method_id(windowForWebExtensionContext:))]
        fn window(
            &self,
            _context: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            tab_of(self.ivars().id).and_then(|(w, _, _)| window_obj(w.id))
        }

        #[unsafe(method(indexInWindowForWebExtensionContext:))]
        fn index(&self, _context: &WKWebExtensionContext) -> NSUInteger {
            tab_of(self.ivars().id).map_or(NSUInteger::MAX, |(_, i, _)| i)
        }

        #[unsafe(method_id(webViewForWebExtensionContext:))]
        fn web_view(&self, _context: &WKWebExtensionContext) -> Option<Retained<WKWebView>> {
            crate::with(|a| a.view(self.ivars().id)).flatten()
        }

        #[unsafe(method_id(titleForWebExtensionContext:))]
        fn title(&self, _context: &WKWebExtensionContext) -> Option<Retained<NSString>> {
            tab_of(self.ivars().id).map(|(_, _, t)| NSString::from_str(&t.title))
        }

        #[unsafe(method_id(urlForWebExtensionContext:))]
        fn url(&self, _context: &WKWebExtensionContext) -> Option<Retained<NSURL>> {
            tab_of(self.ivars().id)
                .and_then(|(_, _, t)| NSURL::URLWithString(&NSString::from_str(&t.url)))
        }

        #[unsafe(method(isSelectedForWebExtensionContext:))]
        fn selected(&self, _context: &WKWebExtensionContext) -> bool {
            tab_of(self.ivars().id).is_some_and(|(w, _, t)| w.active == Some(t.id))
        }

        #[unsafe(method(isPinnedForWebExtensionContext:))]
        fn pinned(&self, _context: &WKWebExtensionContext) -> bool {
            tab_of(self.ivars().id).is_some_and(|(_, _, t)| t.pinned)
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MadoExtensionControllerDelegate"]
    pub struct ControllerDelegate;

    unsafe impl NSObjectProtocol for ControllerDelegate {}

    unsafe impl WKWebExtensionControllerDelegate for ControllerDelegate {
        #[unsafe(method_id(webExtensionController:openWindowsForExtensionContext:))]
        fn open_windows(
            &self,
            _controller: &WKWebExtensionController,
            _context: &WKWebExtensionContext,
        ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionWindow>>> {
            let ids: Vec<WindowId> =
                with(|h| h.cache.windows.iter().map(|w| w.id).collect()).unwrap_or_default();
            let windows: Vec<_> = ids.into_iter().filter_map(window_obj).collect();
            NSArray::from_retained_slice(&windows)
        }

        #[unsafe(method_id(webExtensionController:focusedWindowForExtensionContext:))]
        fn focused_window(
            &self,
            _controller: &WKWebExtensionController,
            _context: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            with(|h| h.cache.windows.iter().find(|w| w.focused).map(|w| w.id))
                .flatten()
                .and_then(window_obj)
        }

        #[unsafe(method(webExtensionController:openNewTabUsingConfiguration:forExtensionContext:completionHandler:))]
        fn open_tab(
            &self,
            _controller: &WKWebExtensionController,
            configuration: &WKWebExtensionTabConfiguration,
            context: &WKWebExtensionContext,
            completion: &block2::DynBlock<
                dyn Fn(*mut ProtocolObject<dyn WKWebExtensionTab>, *mut NSError),
            >,
        ) {
            // SAFETY: reading the configuration on the main thread.
            let (url, active) = unsafe { (configuration.url(), configuration.shouldBeActive()) };
            if let Some(url) = url.and_then(|u| u.absoluteString()) {
                open(context, url.to_string(), false, active);
            }
            // The tab opens through the core; the extension's callback gets
            // no tab object.
            completion.call((std::ptr::null_mut(), std::ptr::null_mut()));
        }

        #[unsafe(method(webExtensionController:openNewWindowUsingConfiguration:forExtensionContext:completionHandler:))]
        fn open_window(
            &self,
            _controller: &WKWebExtensionController,
            configuration: &WKWebExtensionWindowConfiguration,
            context: &WKWebExtensionContext,
            completion: &block2::DynBlock<
                dyn Fn(*mut ProtocolObject<dyn WKWebExtensionWindow>, *mut NSError),
            >,
        ) {
            // SAFETY: as above.
            let urls = unsafe { configuration.tabURLs() };
            let first = urls
                .iter()
                .find_map(|u| u.absoluteString())
                .map(|s| s.to_string());
            open(
                context,
                first.unwrap_or_else(|| "about:blank".into()),
                true,
                true,
            );
            completion.call((std::ptr::null_mut(), std::ptr::null_mut()));
        }

        #[unsafe(method(webExtensionController:openOptionsPageForExtensionContext:completionHandler:))]
        fn open_options(
            &self,
            _controller: &WKWebExtensionController,
            context: &WKWebExtensionContext,
            completion: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            // SAFETY: a property of a live context, on the main thread.
            if let Some(url) = unsafe { context.optionsPageURL() }.and_then(|u| u.absoluteString())
            {
                open(context, url.to_string(), false, true);
            }
            completion.call((std::ptr::null_mut(),));
        }
    }
);

fn open(context: &WKWebExtensionContext, url: String, new_window: bool, active: bool) {
    // SAFETY: a property of a live context, on the main thread.
    let extension = unsafe { context.uniqueIdentifier() }.to_string();
    crate::delegate::send(EngineEvent::ExtensionOpen {
        extension,
        url,
        new_window,
        active,
    });
}

thread_local! {
    /// A window's frame in screen coordinates, from the adapter's host.
    static HOST_WINDOWS: RefCell<Box<dyn Fn(WindowId) -> Option<objc2_foundation::NSRect>>> =
        RefCell::new(Box::new(|_| None));
}

/// Creates the profile's controller (spec 09 §5.1): persistent per profile
/// UUID, using the profile's website data store. Private and guest
/// profiles get none.
pub(crate) fn install(
    mtm: MainThreadMarker,
    profile: ProfileId,
    ephemeral: bool,
    store: &WKWebsiteDataStore,
    frame: Box<dyn Fn(WindowId) -> Option<objc2_foundation::NSRect>>,
) {
    if ephemeral {
        return;
    }
    let uuid = NSUUID::from_bytes(*profile.as_uuid().as_bytes());
    // SAFETY: constructors and setters on the main thread.
    let controller = unsafe {
        let config = WKWebExtensionControllerConfiguration::configurationWithIdentifier(&uuid, mtm);
        config.setDefaultWebsiteDataStore(Some(store));
        WKWebExtensionController::initWithConfiguration(
            WKWebExtensionController::alloc(mtm),
            &config,
        )
    };
    let delegate: Retained<ControllerDelegate> =
        // SAFETY: a plain NSObject subclass without ivars.
        unsafe { msg_send![ControllerDelegate::alloc(mtm), init] };
    // SAFETY: the delegate is retained by the host for the controller's life.
    unsafe { controller.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    HOST_WINDOWS.with_borrow_mut(|f| *f = frame);
    HOST.with_borrow_mut(|h| {
        *h = Some(Host {
            controller,
            _delegate: delegate,
            contexts: BTreeMap::new(),
            cache: ExtensionWindows::default(),
            windows: BTreeMap::new(),
            tabs: BTreeMap::new(),
        });
    });
}

/// The controller new tab views attach to.
pub(crate) fn controller() -> Option<Retained<WKWebExtensionController>> {
    with(|h| h.controller.clone())
}

fn error_text(e: &NSError) -> String {
    format!("{} {} ({})", e.domain(), e.code(), e.localizedDescription())
}

/// Loads an unpacked extension: its context carries the Chrome ID and the
/// granted permissions and match patterns (09 §5.1).
pub fn load(ext: ExtensionLoad, done: Callback<Result<(), EngineError>>) {
    let Some(mtm) = MainThreadMarker::new() else {
        done(Err(EngineError::Unsupported("off the main thread")));
        return;
    };
    if controller().is_none() {
        done(Err(EngineError::Unsupported(
            "no extensions in this profile",
        )));
        return;
    }
    let url =
        NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&ext.path.to_string_lossy()), true);
    let done = std::cell::RefCell::new(Some(done));
    let block = RcBlock::new(move |extension: *mut WKWebExtension, error: *mut NSError| {
        let Some(done) = done.borrow_mut().take() else {
            return;
        };
        // SAFETY: WebKit passes a valid extension or a valid error.
        let extension = unsafe { Retained::retain(extension) };
        let Some(extension) = extension else {
            // SAFETY: as above.
            let why =
                unsafe { error.as_ref() }.map_or_else(|| "no extension".to_owned(), error_text);
            done(Err(EngineError::Failed(why)));
            return;
        };
        done(attach(mtm, &extension, &ext));
    });
    // SAFETY: a folder URL and a block WebKit calls once on the main thread.
    unsafe { WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &block, mtm) };
}

fn attach(
    mtm: MainThreadMarker,
    extension: &WKWebExtension,
    ext: &ExtensionLoad,
) -> Result<(), EngineError> {
    let Some(controller) = controller() else {
        return Err(EngineError::Unsupported("no extensions in this profile"));
    };
    // SAFETY: WebKit objects used on the main thread, where this runs.
    let context = unsafe {
        let context = WKWebExtensionContext::contextForExtension(extension);
        context.setUniqueIdentifier(&NSString::from_str(&ext.id));
        let granted = WKWebExtensionContextPermissionStatus::GrantedExplicitly;
        for p in &ext.permissions {
            context.setPermissionStatus_forPermission(granted, &NSString::from_str(p));
        }
        for h in &ext.hosts {
            let pattern = if h == "<all_urls>" {
                Some(WKWebExtensionMatchPattern::allURLsMatchPattern(mtm))
            } else {
                WKWebExtensionMatchPattern::matchPatternWithString(&NSString::from_str(h), mtm)
            };
            if let Some(pattern) = pattern {
                context.setPermissionStatus_forMatchPattern(granted, &pattern);
            }
        }
        controller
            .loadExtensionContext_error(&context)
            .map_err(|e| EngineError::Failed(error_text(&e)))?;
        context
    };
    with(|h| h.contexts.insert(ext.id.clone(), context));
    Ok(())
}

pub fn unload(id: &str) {
    let Some((controller, context)) =
        with(|h| Some((h.controller.clone(), h.contexts.remove(id)?))).flatten()
    else {
        return;
    };
    // SAFETY: a loaded context, on the main thread.
    if let Err(e) = unsafe { controller.unloadExtensionContext_error(&context) } {
        tracing::warn!(extension = id, error = %error_text(&e), "an extension did not unload");
    }
}

/// The core's windows and tabs changed: update the objects and tell the
/// controller what changed.
pub fn set_windows(next: ExtensionWindows) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some((controller, old)) = with(|h| {
        (
            h.controller.clone(),
            std::mem::replace(&mut h.cache, next.clone()),
        )
    }) else {
        return;
    };
    let new_window = |id: WindowId| {
        let obj = MadoExtensionWindow::alloc(mtm).set_ivars(WindowIvars { id });
        // SAFETY: NSObject's init on a fresh allocation.
        let obj: Retained<MadoExtensionWindow> = unsafe { msg_send![super(obj), init] };
        obj
    };
    let new_tab = |id: TabId| {
        let obj = MadoExtensionTab::alloc(mtm).set_ivars(TabIvars { id });
        // SAFETY: as above.
        let obj: Retained<MadoExtensionTab> = unsafe { msg_send![super(obj), init] };
        obj
    };
    let old_tabs: BTreeMap<TabId, (WindowId, ExtensionTab)> = old
        .windows
        .iter()
        .flat_map(|w| w.tabs.iter().map(move |t| (t.id, (w.id, t.clone()))))
        .collect();
    let new_tabs: BTreeMap<TabId, (WindowId, ExtensionTab)> = next
        .windows
        .iter()
        .flat_map(|w| w.tabs.iter().map(move |t| (t.id, (w.id, t.clone()))))
        .collect();
    // SAFETY: controller notifications on the main thread with live objects.
    unsafe {
        for w in &next.windows {
            if with(|h| h.windows.contains_key(&w.id)) == Some(false) {
                let obj = new_window(w.id);
                with(|h| h.windows.insert(w.id, obj.clone()));
                controller.didOpenWindow(ProtocolObject::from_ref(&*obj));
            }
        }
        for (id, (_, tab)) in &new_tabs {
            match old_tabs.get(id) {
                None => {
                    let obj = new_tab(*id);
                    with(|h| h.tabs.insert(*id, obj.clone()));
                    controller.didOpenTab(ProtocolObject::from_ref(&*obj));
                }
                Some((_, before)) => {
                    let mut changed = WKWebExtensionTabChangedProperties::None;
                    if before.url != tab.url {
                        changed |= WKWebExtensionTabChangedProperties::URL;
                    }
                    if before.title != tab.title {
                        changed |= WKWebExtensionTabChangedProperties::Title;
                    }
                    if before.pinned != tab.pinned {
                        changed |= WKWebExtensionTabChangedProperties::Pinned;
                    }
                    if changed != WKWebExtensionTabChangedProperties::None
                        && let Some(obj) = tab_obj(*id)
                    {
                        controller.didChangeTabProperties_forTab(changed, &obj);
                    }
                }
            }
        }
        for (id, (window, _)) in &old_tabs {
            if !new_tabs.contains_key(id)
                && let Some(obj) = with(|h| h.tabs.remove(id)).flatten()
            {
                let closing = !next.windows.iter().any(|w| w.id == *window);
                controller.didCloseTab_windowIsClosing(ProtocolObject::from_ref(&*obj), closing);
            }
        }
        for w in &old.windows {
            if !next.windows.iter().any(|n| n.id == w.id)
                && let Some(obj) = with(|h| h.windows.remove(&w.id)).flatten()
            {
                controller.didCloseWindow(ProtocolObject::from_ref(&*obj));
            }
        }
        for w in &next.windows {
            let before = old
                .windows
                .iter()
                .find(|o| o.id == w.id)
                .and_then(|o| o.active);
            if w.active != before
                && let Some(active) = w.active.and_then(tab_obj)
            {
                let previous = before.and_then(tab_obj);
                controller.didActivateTab_previousActiveTab(&active, previous.as_deref());
            }
        }
        let focused = next.windows.iter().find(|w| w.focused).map(|w| w.id);
        if focused != old.windows.iter().find(|w| w.focused).map(|w| w.id) {
            let obj = focused.and_then(window_obj);
            controller.didFocusWindow(obj.as_deref());
        }
    }
}
