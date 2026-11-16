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
    Callback, EngineError, EngineEvent, ExtensionActionInfo, ExtensionLoad, ExtensionOp,
    ExtensionTab, ExtensionWindow, ExtensionWindows,
};
use mado_types::{ProfileId, TabId, WindowId};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSArray, NSError, NSString, NSUInteger, NSURL, NSUUID};
use objc2_web_kit::{
    WKWebExtension, WKWebExtensionAction, WKWebExtensionContext,
    WKWebExtensionContextPermissionStatus, WKWebExtensionController,
    WKWebExtensionControllerConfiguration, WKWebExtensionControllerDelegate,
    WKWebExtensionMatchPattern, WKWebExtensionTab, WKWebExtensionTabChangedProperties,
    WKWebExtensionTabConfiguration, WKWebExtensionWindow, WKWebExtensionWindowConfiguration,
    WKWebExtensionWindowState, WKWebExtensionWindowType, WKWebView, WKWebsiteDataStore,
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

        #[unsafe(method(webExtensionController:sendMessage:toApplicationWithIdentifier:forExtensionContext:replyHandler:))]
        fn send_native(
            &self,
            _controller: &WKWebExtensionController,
            message: &objc2::runtime::AnyObject,
            application: Option<&NSString>,
            context: &WKWebExtensionContext,
            reply: &block2::DynBlock<dyn Fn(*mut objc2::runtime::AnyObject, *mut NSError)>,
        ) {
            native_send(message, application, context, reply.copy());
        }

        #[unsafe(method(webExtensionController:connectUsingMessagePort:forExtensionContext:completionHandler:))]
        fn connect_native(
            &self,
            _controller: &WKWebExtensionController,
            port: &objc2_web_kit::WKWebExtensionMessagePort,
            context: &WKWebExtensionContext,
            completion: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let error = native_connect(port, context).err();
            match error {
                Some(e) => completion.call((Retained::as_ptr(&e).cast_mut(),)),
                None => completion.call((std::ptr::null_mut(),)),
            }
        }

        #[unsafe(method(webExtensionController:presentPopupForAction:forExtensionContext:completionHandler:))]
        fn present_popup(
            &self,
            _controller: &WKWebExtensionController,
            action: &WKWebExtensionAction,
            _context: &WKWebExtensionContext,
            completion: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            present(action);
            completion.call((std::ptr::null_mut(),));
        }

        #[unsafe(method(webExtensionController:didUpdateAction:forExtensionContext:))]
        fn did_update_action(
            &self,
            _controller: &WKWebExtensionController,
            _action: &WKWebExtensionAction,
            _context: &WKWebExtensionContext,
        ) {
            send_actions();
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
    native_host_dirs: Vec<std::path::PathBuf>,
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
    NATIVE_DIRS.with_borrow_mut(|d| *d = native_host_dirs);
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
        context.setInspectable(ext.inspectable);
        controller
            .loadExtensionContext_error(&context)
            .map_err(|e| EngineError::Failed(error_text(&e)))?;
        context
    };
    // SAFETY: a property of a loaded context.
    let base = unsafe { context.baseURL() }
        .absoluteString()
        .map(|s| s.to_string())
        .unwrap_or_default();
    with(|h| h.contexts.insert(ext.id.clone(), context));
    crate::delegate::send(EngineEvent::ExtensionLoaded {
        id: ext.id.clone(),
        base_url: base,
    });
    send_actions();
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
    let active = |c: &ExtensionWindows| c.windows.iter().find(|w| w.focused).and_then(|w| w.active);
    if active(&next) != active(&old) {
        send_actions();
    }
}

thread_local! {
    /// Where the next popup goes: the window and the clicked button.
    static ANCHOR: RefCell<Option<(WindowId, mado_types::state::Rect)>> = const { RefCell::new(None) };
}

/// The focused window's active tab.
fn active_tab() -> Option<TabId> {
    with(|h| {
        h.cache
            .windows
            .iter()
            .find(|w| w.focused)
            .and_then(|w| w.active)
    })
    .flatten()
}

fn png_url(image: &objc2_app_kit::NSImage) -> Option<String> {
    use base64::Engine as _;
    let tiff = image.TIFFRepresentation()?;
    let rep = objc2_app_kit::NSBitmapImageRep::imageRepWithData(&tiff)?;
    let props = objc2_foundation::NSDictionary::new();
    // SAFETY: encoding a bitmap we hold.
    let data = unsafe {
        rep.representationUsingType_properties(objc2_app_kit::NSBitmapImageFileType::PNG, &props)
    }?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(crate::convert::bytes(&data))
    ))
}

/// Reports every extension's action on the focused window's active tab.
fn send_actions() {
    let Some(tab) = active_tab() else { return };
    let Some(obj) = tab_obj(tab) else { return };
    let contexts: Vec<(String, Retained<WKWebExtensionContext>)> = with(|h| {
        h.contexts
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    })
    .unwrap_or_default();
    let actions = contexts
        .iter()
        .filter_map(|(id, context)| {
            // SAFETY: WebKit objects on the main thread.
            unsafe {
                let action = context.actionForTab(Some(&obj))?;
                let icon = action
                    .iconForSize(objc2_foundation::NSSize::new(32.0, 32.0))
                    .and_then(|i| png_url(&i));
                Some(ExtensionActionInfo {
                    id: id.clone(),
                    label: action.label().to_string(),
                    badge: action.badgeText().to_string(),
                    icon,
                    has_popup: action.presentsPopup(),
                    enabled: action.isEnabled(),
                })
            }
        })
        .collect();
    crate::delegate::send(EngineEvent::ExtensionActions { tab, actions });
}

/// Shows an action's popup in WebKit's popover under the clicked button
/// (spec 09 §6). WebKit sizes it to its content within Chrome's limits.
fn present(action: &WKWebExtensionAction) {
    let anchor = ANCHOR.with_borrow_mut(Option::take);
    // SAFETY: the action's popover, on the main thread.
    let Some(popover) = (unsafe { action.popupPopover() }) else {
        tracing::warn!("an extension popup has no popover");
        return;
    };
    let window = anchor
        .map(|(w, _)| w)
        .or_else(|| with(|h| h.cache.windows.iter().find(|w| w.focused).map(|w| w.id)).flatten());
    let Some(content) = window.and_then(|w| crate::with(|a| a.content(w)).flatten()) else {
        return;
    };
    let rect = anchor.map_or_else(
        || {
            let b = content.bounds();
            objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(b.size.width - 40.0, b.size.height - 40.0),
                objc2_foundation::NSSize::new(24.0, 24.0),
            )
        },
        |(_, r)| crate::convert::frame_in(&content, r),
    );
    popover.showRelativeToRect_ofView_preferredEdge(
        rect,
        &content,
        objc2_foundation::NSRectEdge::MinY,
    );
    // SAFETY: the action's popup web view, on the main thread.
    let view = unsafe { action.popupWebView() };
    tracing::info!(
        shown = popover.isShown(),
        has_view = view.is_some(),
        "extension popup"
    );
    POPUPS.with_borrow_mut(|p| *p = view);
}

thread_local! {
    /// The last popup's web view, for tests.
    static POPUPS: RefCell<Option<Retained<WKWebView>>> = const { RefCell::new(None) };
}

/// The last popup's page URL, once it has one, for end-to-end tests.
pub fn shown_popup_url() -> Option<String> {
    POPUPS.with_borrow(|p| {
        let web = p.as_ref()?;
        // SAFETY: the URL of a live web view.
        unsafe { web.URL() }
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
    })
}

type NativeReply = RcBlock<dyn Fn(*mut objc2::runtime::AnyObject, *mut NSError)>;

thread_local! {
    static NATIVE_DIRS: RefCell<Vec<std::path::PathBuf>> = const { RefCell::new(Vec::new()) };
    /// `sendNativeMessage` replies waiting for their host, by token.
    static NATIVE_REPLIES: RefCell<BTreeMap<u64, NativeReply>> = const { RefCell::new(BTreeMap::new()) };
    /// Open `connectNative` ports, by token.
    static NATIVE_PORTS: RefCell<BTreeMap<u64, (Retained<objc2_web_kit::WKWebExtensionMessagePort>, mado_platform::native_messaging::Port)>> = const { RefCell::new(BTreeMap::new()) };
    static NATIVE_NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

static NATIVE_LIMITS: std::sync::LazyLock<mado_platform::native_messaging::Limits> =
    std::sync::LazyLock::new(Default::default);

fn native_error(e: &mado_platform::native_messaging::NativeError) -> Retained<NSError> {
    tracing::warn!(error = %e, "native messaging failed");
    NSError::new(1, objc2_foundation::ns_string!("MadoNativeMessaging"))
}

fn native_host(
    application: Option<&NSString>,
    context: &WKWebExtensionContext,
) -> Result<
    (String, mado_platform::native_messaging::Host),
    mado_platform::native_messaging::NativeError,
> {
    // SAFETY: a property of a loaded context.
    let extension = unsafe { context.uniqueIdentifier() }.to_string();
    let name = application.map(|a| a.to_string()).unwrap_or_default();
    let host =
        NATIVE_DIRS.with_borrow(|d| mado_platform::native_messaging::find(d, &name, &extension))?;
    Ok((extension, host))
}

/// `runtime.sendNativeMessage` (09 §9.2): the host runs on its own thread
/// and the reply comes back on the main thread.
fn native_send(
    message: &objc2::runtime::AnyObject,
    application: Option<&NSString>,
    context: &WKWebExtensionContext,
    reply: NativeReply,
) {
    let (extension, host) = match native_host(application, context) {
        Ok(h) => h,
        Err(e) => {
            reply.call((
                std::ptr::null_mut(),
                Retained::as_ptr(&native_error(&e)).cast_mut(),
            ));
            return;
        }
    };
    let message = crate::convert::to_json(Some(message));
    let token = NATIVE_NEXT.with(|n| {
        let t = n.get();
        n.set(t + 1);
        t
    });
    NATIVE_REPLIES.with_borrow_mut(|r| r.insert(token, reply));
    let Some(runner) = crate::delegate::main_runner() else {
        return;
    };
    std::thread::spawn(move || {
        let result =
            mado_platform::native_messaging::send_once(&NATIVE_LIMITS, &host, &extension, &message);
        runner(Box::new(move || {
            let Some(reply) = NATIVE_REPLIES.with_borrow_mut(|r| r.remove(&token)) else {
                return;
            };
            match result.as_ref().map(crate::convert::from_json) {
                Ok(Some(value)) => {
                    reply.call((Retained::as_ptr(&value).cast_mut(), std::ptr::null_mut()))
                }
                Ok(None) => reply.call((std::ptr::null_mut(), std::ptr::null_mut())),
                Err(e) => reply.call((
                    std::ptr::null_mut(),
                    Retained::as_ptr(&native_error(e)).cast_mut(),
                )),
            }
        }));
    });
}

/// `runtime.connectNative` (09 §9.2): one host per port; messages each
/// way until either side closes, then the host is stopped.
fn native_connect(
    port: &objc2_web_kit::WKWebExtensionMessagePort,
    context: &WKWebExtensionContext,
) -> Result<(), Retained<NSError>> {
    // SAFETY: a property of a live port.
    let application = unsafe { port.applicationIdentifier() };
    let (extension, host) =
        native_host(application.as_deref(), context).map_err(|e| native_error(&e))?;
    let token = NATIVE_NEXT.with(|n| {
        let t = n.get();
        n.set(t + 1);
        t
    });
    let Some(runner) = crate::delegate::main_runner() else {
        return Err(NSError::new(
            2,
            objc2_foundation::ns_string!("MadoNativeMessaging"),
        ));
    };
    let native =
        mado_platform::native_messaging::Port::open(&NATIVE_LIMITS, &host, &extension, move |m| {
            runner(Box::new(move || {
                let Some(port) =
                    NATIVE_PORTS.with_borrow(|p| p.get(&token).map(|(port, _)| port.clone()))
                else {
                    return;
                };
                match m {
                    // SAFETY: sending on a live port, on the main thread.
                    Some(value) => unsafe {
                        let value = crate::convert::from_json(&value);
                        port.sendMessage_completionHandler(value.as_deref(), None);
                    },
                    None => {
                        NATIVE_PORTS.with_borrow_mut(|p| p.remove(&token));
                        // SAFETY: as above.
                        unsafe { port.disconnect() };
                    }
                }
            }));
        })
        .map_err(|e| native_error(&e))?;
    let on_message = RcBlock::new(
        move |message: *mut objc2::runtime::AnyObject, _e: *mut NSError| {
            // SAFETY: WebKit hands a message object or null.
            let value = crate::convert::to_json(unsafe { message.as_ref() });
            NATIVE_PORTS.with_borrow(|p| {
                if let Some((_, native)) = p.get(&token)
                    && let Err(e) = native.send(&value)
                {
                    tracing::warn!(error = %e, "a native port message was not sent");
                }
            });
        },
    );
    let on_disconnect = RcBlock::new(move |_e: *mut NSError| {
        if let Some((_, native)) = NATIVE_PORTS.with_borrow_mut(|p| p.remove(&token)) {
            native.close();
        }
    });
    // SAFETY: setting a live port's handlers on the main thread.
    unsafe {
        port.setMessageHandler(Some(&on_message));
        port.setDisconnectHandler(Some(&on_disconnect));
    }
    NATIVE_PORTS.with_borrow_mut(|p| p.insert(token, (objc2::Message::retain(port), native)));
    Ok(())
}

fn context_of(id: &str) -> Option<Retained<WKWebExtensionContext>> {
    with(|h| h.contexts.get(id).cloned()).flatten()
}

/// A call from the core on one extension.
pub fn run(op: ExtensionOp) {
    match op {
        ExtensionOp::Action {
            id,
            window,
            tab,
            anchor,
        } => {
            let Some(context) = context_of(&id) else {
                return;
            };
            ANCHOR.with_borrow_mut(|a| *a = Some((window, anchor)));
            let obj = tab_obj(tab);
            // SAFETY: a loaded context and a tab object, on the main thread.
            unsafe { context.performActionForTab(obj.as_deref()) };
        }
        ExtensionOp::Errors { id, reply } => {
            let errors = context_of(&id)
                .map(|c| {
                    // SAFETY: a property of a loaded context.
                    unsafe { c.errors() }
                        .iter()
                        .map(|e| error_text(&e))
                        .collect()
                })
                .unwrap_or_default();
            reply(errors);
        }
        ExtensionOp::DeleteData { id } => {
            let (Some(context), Some(mtm)) = (context_of(&id), MainThreadMarker::new()) else {
                return;
            };
            let Some(controller) = controller() else {
                return;
            };
            // SAFETY: WebKit calls on the main thread; the blocks run there.
            unsafe {
                let types = WKWebExtensionController::allExtensionDataTypes(mtm);
                let remover = controller.clone();
                let kinds = types.clone();
                let got = RcBlock::new(
                    move |record: *mut objc2_web_kit::WKWebExtensionDataRecord| {
                        let Some(record) = Retained::retain(record) else {
                            return;
                        };
                        let records = NSArray::from_retained_slice(&[record]);
                        let done = RcBlock::new(|| {});
                        remover.removeDataOfTypes_fromDataRecords_completionHandler(
                            &kinds, &records, &done,
                        );
                    },
                );
                controller.fetchDataRecordOfTypes_forExtensionContext_completionHandler(
                    &types, &context, &got,
                );
            }
        }
        ExtensionOp::Inspectable { id, on } => {
            if let Some(c) = context_of(&id) {
                // SAFETY: as above.
                unsafe { c.setInspectable(on) };
            }
        }
    }
}
