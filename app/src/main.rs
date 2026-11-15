//! The Mado application: the only binary users launch (spec 01 §1).
//!
//! A launch parses its arguments, picks the profile, and either becomes that
//! profile's owner (lock, core, control endpoint, windows) or forwards the
//! request to the process that already owns it and exits.

mod args;
#[cfg(debug_assertions)]
mod dev;
mod ipc;
mod lock;
mod profile_service;
mod profiles;
mod update;
mod windows;

use std::process::ExitCode;
use std::sync::{Arc, Mutex, OnceLock};

use mado_control::{Endpoint, ForwardHandler, LaunchSecret};
use mado_core::{CoreConfig, CoreHandle, Input, Internal};
use mado_platform::Paths;
use mado_settings::{Settings, Strings};
use mado_storage::{Kind, Storage, open};
use mado_types::command::{Command, LaunchRequest, Origin};
use mado_types::state::Bounds;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

use crate::args::Args;
use crate::ipc::{Shared, TauriUi};
use crate::lock::ProfileLock;
use crate::profiles::{Choice, Profile};
use crate::windows::{TauriWindows, window_of};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn now() -> i64 {
    mado_core::system_clock()
}

/// What an owner holds until it quits.
struct Owner {
    profile: Profile,
    _lock: ProfileLock,
    paths: Paths,
    app_db: Arc<Storage>,
    core: Arc<OnceLock<CoreHandle>>,
    core_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// A downloaded update, installed on Relaunch or at quit.
    staged: update::Staged,
    /// Opened before the UI starts, so a failure is reported and exits
    /// instead of aborting inside Tauri's setup; taken by `start_owner`.
    ready: Mutex<Option<Ready>>,
}

/// The fallible part of becoming an owner, done before Tauri starts.
struct Ready {
    profile_db: Storage,
    core_app_db: Storage,
    settings: Settings,
    endpoint: Endpoint,
    secret: LaunchSecret,
}

fn prepare(paths: &Paths, id: mado_types::ProfileId) -> Result<Ready, Box<dyn std::error::Error>> {
    let profile_db = open(
        &paths.profile_db(id),
        Kind::Profile,
        &paths.backups(id),
        &paths.broken(id),
    )?;
    if let Some(recovery) = &profile_db.recovery {
        tracing::warn!(?recovery, "the profile database was restored");
    }
    let profile_db = Storage::start(&profile_db)?;
    let settings = {
        let app_conn = rusqlite::Connection::open(paths.app_db())?;
        let profile_conn = rusqlite::Connection::open(paths.profile_db(id))?;
        let (settings, fixes) =
            Settings::load(&app_conn, &profile_conn, Strings::english(), now())?;
        if !fixes.is_empty() {
            mado_settings::persist(&fixes, mado_settings::types::Scope::App, &app_conn)?;
            mado_settings::persist(&fixes, mado_settings::types::Scope::Profile, &profile_conn)?;
        }
        settings
    };
    // The core takes its own app database handle; startup bookkeeping uses ours.
    let core_app_db = Storage::start(&open(
        &paths.app_db(),
        Kind::App,
        &paths.root().join("Backups"),
        &paths.root().join("Broken"),
    )?)?;
    let secret = LaunchSecret::load_or_create(&paths.launch_secret(id))?;
    let socket = paths.control_socket(id);
    let endpoint = Endpoint::bind(&socket)
        .map_err(|e| format!("control endpoint {}: {e}", socket.display()))?;
    Ok(Ready {
        profile_db,
        core_app_db,
        settings,
        endpoint,
        secret,
    })
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    // `--diagnose keychain-write|keychain-read`: proves a build can read the
    // Keychain item an earlier build wrote, without a prompt (spec 15 §10
    // check 1). Prints JSON and exits; touches no profile.
    if std::env::args().nth(1).as_deref() == Some("--diagnose") {
        return diagnose(std::env::args().nth(2).as_deref());
    }
    let args = match Args::parse(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("mado: {e}");
            return ExitCode::from(2);
        }
    };
    match launch(args) {
        Ok(code) => code,
        Err(e) => {
            tracing::error!(error = %e, "Mado could not start");
            eprintln!("mado: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The app's Tauri context, with the version the bundle was built as.
fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

fn diagnose(what: Option<&str>) -> ExitCode {
    const SERVICE: &str = "app.mado.selftest";
    let version = context().package_info().version.to_string();
    let version = version.as_str();
    let result = match what {
        Some("keychain-write") => mado_platform::keychain::set(SERVICE, "update", version.as_bytes()).map(|()| serde_json::json!({ "wrote": version })),
        Some("keychain-read") => mado_platform::keychain::get_without_prompt(SERVICE, "update")
            .map(|v| serde_json::json!({ "read": v.map(|b| String::from_utf8_lossy(&b).into_owned()), "version": version })),
        _ => {
            eprintln!("mado: --diagnose keychain-write|keychain-read");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(v) => {
            println!("{v}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", serde_json::json!({ "error": e.to_string() }));
            ExitCode::FAILURE
        }
    }
}

fn launch(args: Args) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let paths = Paths::system()?;
    std::fs::create_dir_all(paths.root())?;
    let app_db = open(
        &paths.app_db(),
        Kind::App,
        &paths.root().join("Backups"),
        &paths.root().join("Broken"),
    )?;
    let app_db = Arc::new(Storage::start(&app_db)?);
    let profiles = profiles::ensure_one(&app_db, now())?;
    if (args.private || args.guest) && args.profile.is_none() {
        return launch_temporary(&paths, app_db, args, profiles.len());
    }
    let choice = profiles::choose(&app_db, &args, &profiles)?;
    let owner = match choice {
        Choice::Run(profile) => match claim(&paths, &app_db, &args, profile)? {
            Claim::Owner(owner) => Some(owner),
            Claim::Forwarded => return Ok(ExitCode::SUCCESS),
        },
        Choice::Picker(_) => None,
    };
    run_app(paths, app_db, args, owner, profiles.len())
}

/// A private or guest window (spec 12 §1.6): a temporary profile in the
/// temporary folder with an in-memory website data store, deleted on
/// close. Private windows share one such profile while it runs.
fn launch_temporary(
    paths: &Paths,
    app_db: Arc<Storage>,
    args: Args,
    profile_count: usize,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    use profiles::Temporary;
    let temp = paths.temporary();
    std::fs::create_dir_all(temp.profiles())?;
    let kind = if args.guest {
        Temporary::Guest
    } else {
        Temporary::Private
    };
    let marker = |id: mado_types::ProfileId| temp.profile(id).join("kind");
    let mut running_private = None;
    // Sweep folders no process owns (a crash), and find the private one.
    for entry in std::fs::read_dir(temp.profiles())?.flatten() {
        let Some(id) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<mado_types::ProfileId>().ok())
        else {
            continue;
        };
        if !lock::is_held(&temp.lock(id)) {
            let _ = std::fs::remove_dir_all(entry.path());
        } else if std::fs::read_to_string(marker(id)).is_ok_and(|k| k == "private") {
            running_private = Some(id);
        }
    }
    let id = match (kind, running_private) {
        (Temporary::Private, Some(id)) => id,
        _ => {
            let id = mado_types::ProfileId::new();
            std::fs::create_dir_all(temp.profile(id))?;
            std::fs::write(
                marker(id),
                if kind == Temporary::Guest {
                    "guest"
                } else {
                    "private"
                },
            )?;
            id
        }
    };
    let profile = Profile {
        id,
        name: if kind == Temporary::Guest {
            "Guest"
        } else {
            "Private"
        }
        .into(),
        color: "grey".into(),
        avatar: String::new(),
        temporary: Some(kind),
    };
    // A second private launch opens a window in the running one.
    let mut args = args;
    args.new_window = true;
    let owner = match claim(&temp, &app_db, &args, profile)? {
        Claim::Owner(owner) => Some(owner),
        Claim::Forwarded => return Ok(ExitCode::SUCCESS),
    };
    run_app(temp, app_db, args, owner, profile_count)
}

enum Claim {
    Owner(Box<Owner>),
    Forwarded,
}

/// Becomes the profile's owner, or hands the launch to the owner (spec 01
/// §1.1 step 3, §1.3).
fn claim(
    paths: &Paths,
    app_db: &Arc<Storage>,
    args: &Args,
    profile: Profile,
) -> Result<Claim, Box<dyn std::error::Error>> {
    for attempt in 0..2 {
        if let Some(lock) = ProfileLock::acquire(&paths.lock(profile.id), VERSION, now())? {
            let ready = prepare(paths, profile.id)?;
            return Ok(Claim::Owner(Box::new(Owner {
                profile,
                _lock: lock,
                paths: paths.clone(),
                app_db: Arc::clone(app_db),
                core: Arc::new(OnceLock::new()),
                core_thread: Mutex::new(None),
                staged: update::Staged::default(),
                ready: Mutex::new(Some(ready)),
            })));
        }
        let secret = LaunchSecret::read(&paths.launch_secret(profile.id))?;
        let request = LaunchRequest {
            urls: args.urls.clone(),
            files: args
                .files
                .iter()
                .map(|f| f.to_string_lossy().into_owned())
                .collect(),
            new_window: args.new_window,
            private: args.private,
            source: args.source(),
            control: None,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        match runtime.block_on(mado_control::forward(
            &paths.control_socket(profile.id),
            &secret,
            &request,
        )) {
            Ok(()) => return Ok(Claim::Forwarded),
            // The owner may have just exited: try the lock once more.
            Err(e) if attempt == 0 => {
                tracing::warn!(error = %e, "the owner did not answer; retrying the lock")
            }
            Err(e) => return Err(format!("Mado is not responding ({e})").into()),
        }
    }
    Err("Mado is not responding".into())
}

#[cfg(debug_assertions)]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![ipc::mado_invoke, dev::mado_dev_platform]
}

#[cfg(not(debug_assertions))]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![ipc::mado_invoke]
}

fn menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let quit = MenuItem::with_id(app, "quit", "Quit Mado", true, Some("CmdOrCtrl+Q"))?;
    let app_menu = Submenu::with_items(
        app,
        "Mado",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    #[cfg_attr(
        not(debug_assertions),
        expect(unused_mut, reason = "only debug builds add a menu")
    )]
    let mut menus: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![&app_menu, &edit, &window];
    #[cfg(debug_assertions)]
    let developer = Submenu::with_items(
        app,
        "Developer",
        true,
        &[&MenuItem::with_id(
            app,
            "dev-platform",
            "Platform Test Page",
            true,
            None::<&str>,
        )?],
    )?;
    #[cfg(debug_assertions)]
    menus.push(&developer);
    Menu::with_items(app, &menus)
}

/// The settings window: one per process, label `settings` (spec 06 §9).
fn open_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("settings") {
        return w.set_focus();
    }
    WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Settings")
        .inner_size(880.0, 640.0)
        .build()?;
    Ok(())
}

/// Starts everything an owner runs inside the Tauri app: the core, the
/// control endpoint and the first window.
fn start_owner(
    app: &AppHandle,
    owner: &Owner,
    args: &Args,
    profile_count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let id = owner.profile.id;
    let paths = &owner.paths;
    let Ready {
        profile_db,
        core_app_db,
        settings,
        endpoint,
        secret,
    } = owner
        .ready
        .lock()
        .ok()
        .and_then(|mut r| r.take())
        .ok_or("the owner started twice")?;
    let info = owner.profile.info();
    windows::install_webkit(
        app,
        Arc::clone(&owner.core),
        id,
        info.is_private || info.is_guest,
        settings.value(mado_settings::lookup("developer.inspectPages").ok_or("registry")?) == true,
    )?;
    // Read before this run marks itself unclean (spec 04 §8).
    let clean_start = core_app_db
        .readers()
        .read(|c| mado_core::store::clean_quit(c, id))
        .unwrap_or(true);
    let (handle, thread) = mado_core::start(CoreConfig {
        profile: owner.profile.info(),
        app_db: core_app_db,
        profile_db,
        backups: paths.backups(id),
        settings,
        engine: TauriWindows::handle(app.clone(), Arc::clone(&owner.core)),
        // Sites treat Mado as Safari (spec 02 §6).
        user_agent: mado_engine_webkit::safari_version()
            .map_or(mado_engine::UserAgent::EngineDefault, |version| {
                mado_engine::UserAgent::Safari { version }
            }),
        clean_start,
        page_scripts: mado_engine::page::bundled(!mado_engine_webkit::has_geolocation_delegate()),
        ui: Box::new(TauriUi { app: app.clone() }),
        updates: Box::new(update::start(
            app.clone(),
            Arc::clone(&owner.core),
            Arc::clone(&owner.app_db),
            Arc::clone(&owner.staged),
        )),
        clock: mado_core::system_clock,
        captive_probe: captive_probe(),
        os: mado_core::OsPermissions::system(),
        profiles: Arc::new(profile_service::AppProfiles {
            app_db: Arc::clone(&owner.app_db),
            paths: owner.paths.clone(),
            own: id,
            removers: profile_service::removers(app.clone()),
            launch: Box::new(profile_service::launch),
            now,
        }),
    })?;
    let _ = owner.core.set(handle.clone());
    let early = EARLY_LINKS
        .lock()
        .map(|mut l| std::mem::take(&mut *l))
        .unwrap_or_default();
    open_links(&handle, early);
    if let Ok(mut slot) = owner.core_thread.lock() {
        *slot = Some(thread);
    }

    // The control endpoint: launch.forward only, while control is off.
    let forward_to = handle.clone();
    let rows = Arc::clone(&owner.app_db);
    let on_forward: ForwardHandler = Arc::new(move |request: LaunchRequest| {
        let core = forward_to.clone();
        let rows = Arc::clone(&rows);
        Box::pin(async move {
            // Another profile asks this one to quit or reload itself
            // (spec 12 §1.2, §1.4); the launch secret was checked.
            match request.control {
                Some(mado_types::command::OwnerAction::Quit) => {
                    core.send(Input::Internal(Internal::Terminate));
                    return Ok(());
                }
                Some(mado_types::command::OwnerAction::ProfileChanged) => {
                    if let Some(p) = profiles::list(&rows)
                        .ok()
                        .and_then(|l| l.into_iter().find(|p| p.id == id))
                    {
                        core.send(Input::Internal(Internal::ProfileChanged(p.info())));
                    }
                    return Ok(());
                }
                None => {}
            }
            core.command(Origin::System, Command::LaunchForward(request))
                .await
                .map_err(|_| {
                    mado_types::MadoError::new(mado_types::ErrorCode::Internal, "error.coreStopped")
                })?
                .map(|_| ())
        })
    });
    tauri::async_runtime::spawn(mado_control::serve(
        endpoint,
        secret,
        on_forward,
        test_access(&handle, app),
    ));
    write_control_entry(owner)?;

    if owner.profile.temporary.is_none() {
        profiles::set_app_state(&owner.app_db, "last_profile_id", &id.to_string()).wait()?;
        profiles::set_app_state(&owner.app_db, &format!("clean_quit.{id}"), "0").wait()?;
    }
    if profile_count > 1 {
        mado_platform::dock::set_badge(Some(&owner.profile.initial()));
    }
    if !args.hidden {
        handle.send(Input::Internal(Internal::Startup {
            bounds: Bounds::default(),
        }));
    }
    // URLs this launch was started with (a link that started Mado, a
    // profile opened for a link): link rules already chose this profile.
    if !args.urls.is_empty() {
        let request = LaunchRequest {
            urls: args.urls.clone(),
            files: Vec::new(),
            new_window: false,
            private: false,
            source: mado_types::command::LaunchSource::Cli,
            control: None,
        };
        let core = handle.clone();
        tauri::async_runtime::spawn(async move {
            let _ = core
                .command(Origin::System, Command::LaunchForward(request))
                .await;
        });
    }
    // Memory pressure and the power source steer tab sleeping (04 §7).
    let pressure = handle.clone();
    mado_platform::power::watch_memory_pressure(move |level| {
        pressure.send(Input::Internal(Internal::MemoryPressure {
            critical: level == mado_platform::power::Pressure::Critical,
        }));
    });
    // A click on a page's notification goes back to its tab (07 §1.1).
    let clicks = handle.clone();
    if let Err(e) = mado_platform::notify::on_click(move |id| {
        clicks.send(Input::Internal(Internal::NotificationClicked { id }));
    }) {
        tracing::warn!(error = %e, "notification clicks are not delivered");
    }
    let power = handle.clone();
    mado_platform::power::watch_power(std::time::Duration::from_secs(60), move |source| {
        power.send(Input::Internal(Internal::Power(mado_core::Power {
            on_battery: source.on_battery,
            percent: source.percent,
        })));
    });
    Ok(())
}

/// End-to-end tests' access to the control endpoint (spec 16 §1): with the
/// `test-hooks` feature and `MADO_TEST_TOKEN` set, a session that says
/// `hello` with that token may send commands as a control client and the
/// `test.*` hooks of spec 16 §3. Release builds never compile this.
#[cfg(feature = "test-hooks")]
fn test_access(core: &CoreHandle, app: &tauri::AppHandle) -> Option<mado_control::TestAccess> {
    use mado_core::TestHook;
    let app = app.clone();
    let token = std::env::var("MADO_TEST_TOKEN")
        .ok()
        .filter(|t| t.len() >= 16)?;
    let core = core.clone();
    let client = mado_types::ClientId::new();
    let call: mado_control::CallHandler =
        Arc::new(move |method: String, params: serde_json::Value| {
            let core = core.clone();
            let app = app.clone();
            Box::pin(async move {
                if method == "test.click" {
                    return test_click(&app, &params).await;
                }
                if method == "test.hasDataStore" {
                    return test_has_data_store(&app, &params).await;
                }
                let hook = match method.as_str() {
                    "test.dropNextPatch" => Some(TestHook::DropNextPatch),
                    "test.fakeAuthSuccess" => Some(TestHook::FakeAuthSuccess),
                    "test.fakeClock" => params
                        .get("now")
                        .and_then(serde_json::Value::as_i64)
                        .map(TestHook::FakeClock),
                    _ => None,
                };
                if let Some(hook) = hook {
                    core.send(Input::Internal(Internal::Hook(hook)));
                    return Ok(serde_json::Value::Null);
                }
                // Runs a command as Mado's own settings window would, for
                // commands control clients may never send (profiles).
                let (origin, params) = if method == "test.asUser" {
                    (Origin::Settings, params["command"].clone())
                } else {
                    (Origin::Control { client }, params)
                };
                let method = if method == "test.asUser" {
                    params["type"].as_str().unwrap_or_default().to_owned()
                } else {
                    method
                };
                let mut body = if params.is_object() {
                    params
                } else {
                    serde_json::json!({})
                };
                body["type"] = serde_json::json!(method);
                let cmd: Command = serde_json::from_value(body).map_err(|e| {
                    mado_types::MadoError::new(
                        mado_types::ErrorCode::InvalidInput,
                        "error.control.badCommand",
                    )
                    .arg("reason", e.to_string())
                })?;
                core.command(origin, cmd).await.map_err(|_| {
                    mado_types::MadoError::new(mado_types::ErrorCode::Internal, "error.coreStopped")
                })?
            })
        });
    Some(mado_control::TestAccess { token, call })
}

/// `test.click { tabId, x, y, command?, shift?, option? }`: a real AppKit
/// click on a tab's page (spec 16 §3).
#[cfg(feature = "test-hooks")]
async fn test_click(
    app: &tauri::AppHandle,
    params: &serde_json::Value,
) -> Result<serde_json::Value, mado_types::MadoError> {
    let bad = |reason: &str| {
        mado_types::MadoError::new(
            mado_types::ErrorCode::InvalidInput,
            "error.control.badCommand",
        )
        .arg("reason", reason.to_owned())
    };
    let tab: mado_types::TabId =
        serde_json::from_value(params["tabId"].clone()).map_err(|e| bad(&e.to_string()))?;
    let x = params["x"].as_f64().ok_or_else(|| bad("x"))?;
    let y = params["y"].as_f64().ok_or_else(|| bad("y"))?;
    let flag = |k: &str| params[k].as_bool().unwrap_or(false);
    let modifiers = mado_engine::tab::Modifiers {
        command: flag("command"),
        shift: flag("shift"),
        option: flag("option"),
        control: false,
        button: 0,
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(mado_engine_webkit::synthesize_click(tab, x, y, modifiers));
    })
    .map_err(|e| bad(&e.to_string()))?;
    rx.await
        .map_err(|_| bad("the main thread dropped the click"))?
        .map_err(|e| bad(&e.to_string()))?;
    Ok(serde_json::Value::Null)
}

/// `test.hasDataStore { profileId }`: whether WebKit still has that
/// profile's website data store (spec 12 §7 check 2).
#[cfg(feature = "test-hooks")]
async fn test_has_data_store(
    app: &tauri::AppHandle,
    params: &serde_json::Value,
) -> Result<serde_json::Value, mado_types::MadoError> {
    let bad = |reason: String| {
        mado_types::MadoError::new(
            mado_types::ErrorCode::InvalidInput,
            "error.control.badCommand",
        )
        .arg("reason", reason)
    };
    let id: mado_types::ProfileId =
        serde_json::from_value(params["profileId"].clone()).map_err(|e| bad(e.to_string()))?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        mado_engine_webkit::has_data_store(
            id,
            Box::new(move |found| {
                let _ = tx.send(found);
            }),
        );
    })
    .map_err(|e| bad(e.to_string()))?;
    Ok(serde_json::json!(rx.await.map_err(|e| bad(e.to_string()))?))
}

#[cfg(not(feature = "test-hooks"))]
fn test_access(_core: &CoreHandle, _app: &tauri::AppHandle) -> Option<mado_control::TestAccess> {
    None
}

/// `<root>/Control/<ProfileId>.json`, written atomically (spec 14 §1).
fn write_control_entry(owner: &Owner) -> std::io::Result<()> {
    let id = owner.profile.id;
    let path = owner.paths.control_entry(id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let entry = serde_json::json!({
        "profileId": id, "name": owner.profile.name,
        "endpoint": owner.paths.control_socket(id), "pid": std::process::id(), "startedAt": now(),
    });
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, entry.to_string())?;
    std::fs::rename(tmp, path)
}

/// The quit sequence available in M1 (spec 01 §1.6): stop the core after
/// its queued writes, record the clean quit, remove the control entry.
fn shutdown(owner: &Owner) {
    // An update downloaded but never relaunched into: install it now, so
    // the next launch runs it.
    update::install_staged(&owner.staged);
    if let Some(core) = owner.core.get() {
        let (done, rx) = tokio::sync::oneshot::channel();
        if core.send(Input::Internal(Internal::Shutdown { done })) {
            let _ = rx.blocking_recv();
        }
    }
    if let Some(thread) = owner.core_thread.lock().ok().and_then(|mut t| t.take()) {
        let _ = thread.join();
    }
    let id = owner.profile.id;
    let _ = std::fs::remove_file(owner.paths.control_entry(id));
    if owner.profile.temporary.is_some() {
        // Nothing of a private or guest session stays (spec 12 §1.6).
        match std::fs::remove_dir_all(owner.paths.profile(id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                tracing::error!(error = %e, "the temporary profile was not deleted");
            }
            _ => {}
        }
        return;
    }
    if let Err(e) = profiles::set_app_state(&owner.app_db, &format!("clean_quit.{id}"), "1").wait()
    {
        tracing::error!(error = %e, "the clean-quit marker was not written");
    }
}

fn run_app(
    paths: Paths,
    app_db: Arc<Storage>,
    args: Args,
    owner: Option<Box<Owner>>,
    profile_count: usize,
) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let core = owner
        .as_ref()
        .map_or_else(|| Arc::new(OnceLock::new()), |o| Arc::clone(&o.core));
    let owner = Arc::new(Mutex::new(owner));
    let setup_owner = Arc::clone(&owner);
    let setup_args = args.clone();
    let quitting = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let quit_flag = Arc::clone(&quitting);
    let term_flag = Arc::clone(&quitting);
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Shared {
            core,
            quitting: Arc::clone(&quitting),
        })
        .invoke_handler(handlers())
        .menu(menu)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "settings" => {
                if let Err(e) = open_settings(app) {
                    tracing::error!(error = %e, "the settings window did not open");
                }
            }
            #[cfg(debug_assertions)]
            "dev-platform" => {
                if let Err(e) = dev::open(app) {
                    tracing::error!(error = %e, "the platform test page did not open");
                }
            }
            "quit" => {
                // The core asks about typed text and a second ⌘Q (04 §9),
                // then calls back through `UiSink::quit`.
                let core = app
                    .try_state::<Shared>()
                    .and_then(|s| s.core.get().cloned());
                match core {
                    Some(core) => {
                        core.send(Input::Internal(Internal::QuitRequested { shortcut: true }));
                    }
                    None => {
                        quit_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        app.exit(0);
                    }
                }
            }
            _ => {}
        })
        .setup(move |app| {
            // SIGTERM (kill, logout tools) quits like ⌘Q, so the clean-quit
            // marker and the database checkpoint still happen. The core first
            // pauses running downloads, without asking about typed text.
            let handle = app.handle().clone();
            let flag = Arc::clone(&term_flag);
            tauri::async_runtime::spawn(async move {
                if let Ok(mut term) =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    && term.recv().await.is_some()
                {
                    let core = handle
                        .try_state::<Shared>()
                        .and_then(|s| s.core.get().cloned());
                    let asked = core.is_some_and(|c| c.send(Input::Internal(Internal::Terminate)));
                    if !asked {
                        flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        handle.exit(0);
                    }
                }
            });
            let guard = setup_owner.lock().map_err(|_| "owner lock poisoned")?;
            match guard.as_ref() {
                Some(owner) => {
                    if let Err(e) = start_owner(app.handle(), owner, &setup_args, profile_count) {
                        tracing::error!(error = %e, "Mado could not start");
                        eprintln!("mado: {e}");
                        app.handle().exit(1);
                    }
                }
                None => {
                    // The profile picker: placeholder UI until M6 (spec 01 §1.4).
                    WebviewWindowBuilder::new(app, "picker", WebviewUrl::App("picker.html".into()))
                        .title("Choose a profile")
                        .inner_size(420.0, 480.0)
                        .build()?;
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && let Some(id) = window_of(window.label())
                && let Some(shared) = window.try_state::<Shared>()
                && let Some(core) = shared.core.get()
            {
                // The core removes the window from the state, then closes it.
                api.prevent_close();
                core.send(Input::Engine(
                    mado_engine::EngineEvent::WindowCloseRequested { window: id },
                ));
            }
        })
        .build(context())?;
    let _ = (paths, app_db);
    app.run(move |_app, event| match event {
        // Closing the last window keeps Mado running, as Mac apps do.
        RunEvent::ExitRequested {
            api, code: None, ..
        } if !quitting.load(std::sync::atomic::Ordering::SeqCst) => api.prevent_exit(),
        // Links from other apps while Mado runs, or that launched it
        // (macOS sends them as Apple Events, not arguments).
        RunEvent::Opened { urls } => {
            let urls: Vec<String> = urls
                .into_iter()
                .filter(|u| matches!(u.scheme(), "http" | "https"))
                .map(String::from)
                .collect();
            let core = owner
                .lock()
                .ok()
                .and_then(|g| g.as_ref().and_then(|o| o.core.get().cloned()));
            match core {
                Some(core) => open_links(&core, urls),
                None => {
                    if let Ok(mut early) = EARLY_LINKS.lock() {
                        early.extend(urls);
                    }
                }
            }
        }
        RunEvent::Exit => {
            mado_engine_webkit::shutdown();
            if let Ok(guard) = owner.lock()
                && let Some(owner) = guard.as_ref()
            {
                shutdown(owner);
            }
        }
        _ => {}
    });
    Ok(ExitCode::SUCCESS)
}

/// Links that arrived before the core started.
static EARLY_LINKS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Hands links from other apps to the core, which applies link rules
/// (spec 12 §1.3) and opens the rest here (spec 05 §9).
fn open_links(core: &CoreHandle, urls: Vec<String>) {
    if urls.is_empty() {
        return;
    }
    let request = LaunchRequest {
        urls,
        files: Vec::new(),
        new_window: false,
        private: false,
        source: mado_types::command::LaunchSource::Link,
        control: None,
    };
    let core = core.clone();
    tauri::async_runtime::spawn(async move {
        let _ = core
            .command(Origin::System, Command::LaunchForward(request))
            .await;
    });
}

/// Apple's captive portal probe (spec 05 §8). End-to-end tests point it
/// elsewhere, or turn it off with an empty value.
fn captive_probe() -> Option<String> {
    #[cfg(feature = "test-hooks")]
    if let Ok(probe) = std::env::var("MADO_CAPTIVE_PROBE") {
        return (!probe.is_empty()).then_some(probe);
    }
    Some("http://captive.apple.com/hotspot-detect.html".to_owned())
}
