//! The Engine trait, engine events and capabilities shared by every adapter.
//!
//! Specification: `mado-docs/spec/02-engine.md` sections 1 to 3. Roadmap:
//! M2-T01. The blocker's `set_blocker_rules` joins the trait with M3-T11
//! and the extension host's `extensions` with M3-T04.
//!
//! Adapters run on the main thread. The core reaches them through an
//! [`EngineHandle`], which queues [`EngineOp`]s for the main thread, and
//! they report back through an [`EventSink`].

pub mod caps;
pub mod event;
pub mod nav;
pub mod page;
pub mod reply;
pub mod tab;
pub mod window;

use std::sync::Arc;

pub use caps::{ArchiveFormat, Capabilities, KeySystem, PermissionKind};
pub use event::EngineEvent;
pub use nav::{ExternalReason, NavDecision, NavPhase, NavRequest, NetError};
pub use reply::{Callback, Expiry, REPLY_TIMEOUT, Reply};
pub use tab::{NavKind, TabCommand, TabInitial, TabSpec, UserAgent, World};
pub use window::WindowCommand;

use mado_types::{TabId, WindowId};

/// Why the engine could not carry out a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// No tab view with that ID.
    NoTab,
    /// No window with that ID.
    NoWindow,
    /// The engine does not support this (see [`Capabilities`]).
    Unsupported(&'static str),
    /// The engine reported an error.
    Failed(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTab => f.write_str("no such tab view"),
            Self::NoWindow => f.write_str("no such window"),
            Self::Unsupported(what) => write!(f, "the engine does not support {what}"),
            Self::Failed(why) => write!(f, "the engine failed: {why}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// When a page script runs (spec 02 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectAt {
    DocumentStart,
    DocumentEnd,
}

/// One bundled page script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageScript {
    pub name: &'static str,
    pub world: World,
    pub at: InjectAt,
    pub main_frame_only: bool,
    pub source: String,
}

/// The scripts every new tab view gets, besides its own `bridge`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageScriptSet {
    pub scripts: Vec<PageScript>,
}

/// Which kinds of site data to clear (spec 08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DataKinds {
    pub cookies: bool,
    pub cache: bool,
    pub local_storage: bool,
    pub indexed_db: bool,
    pub service_workers: bool,
    pub other_site_data: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClearDataRequest {
    pub kinds: DataKinds,
    /// Unix milliseconds; `None` clears all time.
    pub since: Option<i64>,
    /// Only these hosts; `None` clears every site.
    pub hosts: Option<Vec<String>>,
}

/// The profile's proxy (settings `proxy.*`, spec 02 §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ProxyConfig {
    #[default]
    System,
    Direct,
    Manual {
        /// `host:port`, empty for none.
        http: String,
        socks5: String,
        bypass: Vec<String>,
    },
}

impl ProxyConfig {
    /// From the values of `proxy.mode`, `proxy.http`, `proxy.socks5` and
    /// `proxy.bypass`. An unknown mode is the system proxy.
    pub fn from_settings(mode: &str, http: &str, socks5: &str, bypass: Vec<String>) -> Self {
        match mode {
            "direct" => Self::Direct,
            "manual" => Self::Manual {
                http: http.to_owned(),
                socks5: socks5.to_owned(),
                bypass,
            },
            _ => Self::System,
        }
    }
}

/// The interface the core uses to drive pages (spec 02 §1). Implemented by
/// adapters on the main thread.
pub trait Engine {
    fn capabilities(&self) -> &Capabilities;
    fn create_tab(&mut self, spec: TabSpec) -> Result<(), EngineError>;
    fn destroy_tab(&mut self, tab: TabId);
    fn apply(&mut self, tab: TabId, cmd: TabCommand) -> Result<(), EngineError>;
    fn apply_window(&mut self, window: WindowId, cmd: WindowCommand);
    fn set_page_scripts(&mut self, scripts: PageScriptSet);
    fn clear_data(&mut self, req: ClearDataRequest, done: Callback<Result<(), EngineError>>);
    /// Takes effect for new tab views.
    fn set_proxy(&mut self, proxy: ProxyConfig);
}

/// One call on the engine, queued from the core for the main thread.
pub enum EngineOp {
    CreateTab(TabSpec),
    DestroyTab(TabId),
    Apply(TabId, TabCommand),
    ApplyWindow(WindowId, WindowCommand),
    SetPageScripts(PageScriptSet),
    ClearData(ClearDataRequest, Callback<Result<(), EngineError>>),
    SetProxy(ProxyConfig),
}

impl std::fmt::Debug for EngineOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateTab(spec) => write!(f, "CreateTab({})", spec.tab),
            Self::DestroyTab(tab) => write!(f, "DestroyTab({tab})"),
            Self::Apply(tab, cmd) => write!(f, "Apply({tab}, {cmd:?})"),
            Self::ApplyWindow(window, cmd) => write!(f, "ApplyWindow({window}, {cmd:?})"),
            Self::SetPageScripts(s) => write!(f, "SetPageScripts({} scripts)", s.scripts.len()),
            Self::ClearData(req, _) => write!(f, "ClearData({:?})", req.kinds),
            Self::SetProxy(_) => f.write_str("SetProxy"),
        }
    }
}

impl EngineOp {
    /// Runs the call on `engine`, on the main thread. A tab view that
    /// cannot be created is reported as gone, so the core shows it crashed
    /// with Reload; a command that fails answers its callback with the error.
    pub fn run(self, engine: &mut dyn Engine, events: &EventSink) {
        match self {
            Self::CreateTab(spec) => {
                let tab = spec.tab;
                if let Err(e) = engine.create_tab(spec) {
                    tracing::error!(%tab, error = %e, "a tab view could not be created");
                    events.send(EngineEvent::ProcessGone {
                        tab,
                        reason: event::GoneReason::Crashed,
                    });
                }
            }
            Self::DestroyTab(tab) => engine.destroy_tab(tab),
            Self::Apply(tab, cmd) => {
                let name = cmd.name();
                if let Err(e) = engine.apply(tab, cmd) {
                    tracing::warn!(%tab, command = name, error = %e, "a tab command failed");
                }
            }
            Self::ApplyWindow(window, cmd) => engine.apply_window(window, cmd),
            Self::SetPageScripts(scripts) => engine.set_page_scripts(scripts),
            Self::ClearData(req, done) => engine.clear_data(req, done),
            Self::SetProxy(proxy) => engine.set_proxy(proxy),
        }
    }
}

/// The core's handle on the engine: queues calls for the main thread.
/// Cheap to clone and `Send`.
#[derive(Clone)]
pub struct EngineHandle {
    dispatch: Arc<dyn Fn(EngineOp) + Send + Sync>,
}

impl std::fmt::Debug for EngineHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EngineHandle")
    }
}

impl EngineHandle {
    /// `dispatch` hands each call to the main thread (the app: Tauri's
    /// `run_on_main_thread`; tests: straight to a fake engine).
    pub fn new(dispatch: impl Fn(EngineOp) + Send + Sync + 'static) -> Self {
        Self {
            dispatch: Arc::new(dispatch),
        }
    }

    pub fn send(&self, op: EngineOp) {
        (self.dispatch)(op);
    }
}

/// Where the engine reports events: the core's input channel.
#[derive(Clone)]
pub struct EventSink {
    send: Arc<dyn Fn(EngineEvent) + Send + Sync>,
}

impl std::fmt::Debug for EventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventSink")
    }
}

impl EventSink {
    pub fn new(send: impl Fn(EngineEvent) + Send + Sync + 'static) -> Self {
        Self {
            send: Arc::new(send),
        }
    }

    pub fn send(&self, event: EngineEvent) {
        (self.send)(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_settings_map_to_a_config() {
        assert_eq!(
            ProxyConfig::from_settings("system", "", "", vec![]),
            ProxyConfig::System
        );
        assert_eq!(
            ProxyConfig::from_settings("direct", "h:1", "", vec![]),
            ProxyConfig::Direct
        );
        assert_eq!(
            ProxyConfig::from_settings("manual", "h:1", "s:2", vec!["local".into()]),
            ProxyConfig::Manual {
                http: "h:1".into(),
                socks5: "s:2".into(),
                bypass: vec!["local".into()],
            }
        );
        assert_eq!(
            ProxyConfig::from_settings("bogus", "", "", vec![]),
            ProxyConfig::System
        );
    }
}
