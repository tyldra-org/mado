//! The prototype shell: one Tauri window whose UI webview fills the window,
//! with a native tab view over the page rectangle the UI reports (01 §5.1,
//! §5.2).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tauri::webview::WebviewBuilder;
use tauri::window::WindowBuilder;
use tauri::{AppHandle, LogicalPosition, Manager, WebviewUrl};

#[cfg(target_os = "macos")]
use crate::macos as platform;
#[cfg(windows)]
use crate::windows_tab as platform;

#[cfg(not(any(target_os = "macos", windows)))]
compile_error!("the M0 prototype runs on macOS and Windows only");

pub const USAGE: &str =
    "usage: mado-proto [--url <url>] [--smoke <url> [--out <file>] [--screenshot <png>]]

  --url <url>     first page of the tab view (default https://example.com/)
  --smoke <url>   type <url> into the UI's address field, wait for the tab
                  view to load it, print what loaded and the view stack, quit
  --out <file>    also write the --smoke result as JSON to <file>
  --screenshot <png>  capture the window at the end of --smoke";

pub const WINDOW: &str = "main";
pub const UI_LABEL: &str = "ui-main";
const WINDOW_SIZE: (f64, f64) = (1200.0, 800.0);
/// Where the page starts until the UI reports its real content rectangle.
const INITIAL_PAGE_TOP: f64 = 44.0;
/// How long a scripted step waits before it reports a failure.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct Args {
    pub url: String,
    pub smoke: Option<String>,
    pub out: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
}

impl Args {
    pub fn parse(mut it: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut args = Args {
            url: "https://example.com/".into(),
            smoke: None,
            out: None,
            screenshot: None,
        };
        while let Some(flag) = it.next() {
            let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
            match flag.as_str() {
                "--url" => args.url = normalize_url(&value()?)?,
                "--smoke" => args.smoke = Some(value()?),
                "--out" => args.out = Some(PathBuf::from(value()?)),
                "--screenshot" => args.screenshot = Some(PathBuf::from(value()?)),
                "-h" | "--help" => return Err("help".into()),
                other => return Err(format!("unknown argument {other}")),
            }
        }
        Ok(args)
    }
}

/// A rectangle in the UI webview's CSS pixels, top-left origin.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug)]
struct Shell {
    tab: OnceLock<platform::TabId>,
    /// Fires once, when the UI first reports its content rectangle.
    ui_ready: Mutex<Option<mpsc::Sender<()>>>,
}

impl Shell {
    fn tab(&self) -> Result<platform::TabId, String> {
        self.tab
            .get()
            .copied()
            .ok_or_else(|| "the tab view does not exist yet".into())
    }
}

/// Accepts what a user types in an address field: a full URL, or a host,
/// which gets `https://`.
pub fn normalize_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("empty address".into());
    }
    if input.contains("://") || input.starts_with("about:") || input.starts_with("data:") {
        Ok(input.to_owned())
    } else {
        Ok(format!("https://{input}"))
    }
}

#[tauri::command]
fn navigate(shell: tauri::State<'_, Shell>, url: String) -> Result<(), String> {
    let url = normalize_url(&url)?;
    platform::navigate(shell.tab()?, &url)
}

#[tauri::command]
fn report_content_rect(
    app: AppHandle,
    shell: tauri::State<'_, Shell>,
    rect: Rect,
) -> Result<(), String> {
    let window = app.get_window(WINDOW).ok_or("no main window")?;
    platform::set_frame(&window, shell.tab()?, rect)?;
    let ready = shell
        .ui_ready
        .lock()
        .map_err(|_| "ui_ready lock poisoned")?
        .take();
    if let Some(ready) = ready {
        // The smoke thread may have given up already; nothing to do then.
        let _ = ready.send(());
    }
    Ok(())
}

/// Runs `f` on the main thread and waits for its result.
pub fn on_main<R: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce() -> R + Send + 'static,
) -> Result<R, String> {
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| format!("run_on_main_thread: {e}"))?;
    rx.recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "the main thread did not answer".into())
}

pub fn run(args: Args) -> ExitCode {
    let (ready_tx, ready_rx) = mpsc::channel();
    let Args {
        url,
        smoke,
        out,
        screenshot,
    } = args;
    let built = tauri::Builder::default()
        .manage(Shell {
            tab: OnceLock::new(),
            ui_ready: Mutex::new(Some(ready_tx)),
        })
        .invoke_handler(tauri::generate_handler![navigate, report_content_rect])
        .setup(move |app| {
            let window = WindowBuilder::new(app, WINDOW)
                .title("Mado M0 prototype")
                .inner_size(WINDOW_SIZE.0, WINDOW_SIZE.1)
                .build()?;
            // The UI webview fills the window's content area (01 §5.1).
            let size = window
                .inner_size()?
                .to_logical::<f64>(window.scale_factor()?);
            window.add_child(
                WebviewBuilder::new(UI_LABEL, WebviewUrl::App("index.html".into())).auto_resize(),
                LogicalPosition::new(0.0, 0.0),
                size,
            )?;
            let page = Rect {
                x: 0.0,
                y: INITIAL_PAGE_TOP,
                width: size.width,
                height: size.height - INITIAL_PAGE_TOP,
            };
            let tab = platform::create_tab(&window, page, &url)?;
            app.state::<Shell>()
                .tab
                .set(tab)
                .map_err(|_| "tab created twice")?;
            if let Some(smoke_url) = smoke.clone() {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    run_smoke(
                        &handle,
                        &ready_rx,
                        &smoke_url,
                        out.as_ref(),
                        screenshot.as_deref(),
                    );
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!());
    let app = match built {
        Ok(app) => app,
        Err(e) => {
            eprintln!("mado-proto: {e}");
            return ExitCode::FAILURE;
        }
    };
    let code = app.run_return(|_, _| {});
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn run_smoke(
    app: &AppHandle,
    ready: &mpsc::Receiver<()>,
    url: &str,
    out: Option<&PathBuf>,
    screenshot: Option<&Path>,
) {
    let result = smoke(app, ready, url, screenshot);
    let (code, report) = match result {
        Ok(report) => (0, report),
        Err(error) => (1, json!({ "ok": false, "error": error })),
    };
    let text = serde_json::to_string_pretty(&report).unwrap_or_else(|e| e.to_string());
    println!("{text}");
    if let Some(path) = out
        && let Err(e) = std::fs::write(path, format!("{text}\n"))
    {
        eprintln!("mado-proto: cannot write {}: {e}", path.display());
    }
    app.exit(code);
}

fn smoke(
    app: &AppHandle,
    ready: &mpsc::Receiver<()>,
    url: &str,
    screenshot_path: Option<&Path>,
) -> Result<Value, String> {
    ready
        .recv_timeout(STEP_TIMEOUT)
        .map_err(|_| "the UI never reported its content rect")?;
    let tab = app.state::<Shell>().tab()?;
    // Load the initial page first, so the typed URL is a second navigation.
    platform::wait_navigation(tab, STEP_TIMEOUT)?;
    platform::clear_navigation(tab);
    let ui = app.get_webview(UI_LABEL).ok_or("no UI webview")?;
    let typed = serde_json::to_string(url).map_err(|e| e.to_string())?;
    ui.eval(format!("window.madoProto.typeAndGo({typed})"))
        .map_err(|e| e.to_string())?;
    let loaded = platform::wait_navigation(tab, STEP_TIMEOUT)?;
    let handle = app.clone();
    let views = on_main(app, move || {
        let window = handle.get_window(WINDOW).ok_or("no main window")?;
        platform::describe_views(&window)
    })??;
    let screenshot = match screenshot_path {
        Some(path) => Some(platform::screenshot(app, WINDOW, path)?),
        None => None,
    };
    Ok(json!({
        "ok": true,
        "typed": url,
        "loaded": loaded,
        "views": views,
        "screenshot": screenshot,
        "environment": platform::environment(),
    }))
}
