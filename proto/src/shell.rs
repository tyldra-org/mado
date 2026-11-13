//! The prototype shell: one Tauri window holding, bottom to top, the UI
//! webview filling the window, a native tab view over the page rectangle the
//! UI reports, and a hidden overlay webview (01 §5.1, §5.2).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Condvar, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::webview::WebviewBuilder;
use tauri::window::WindowBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl};

#[cfg(target_os = "macos")]
use crate::macos as platform;
#[cfg(windows)]
use crate::windows_tab as platform;

#[cfg(not(any(target_os = "macos", windows)))]
compile_error!("the M0 prototype runs on macOS and Windows only");

pub const USAGE: &str = "usage: mado-proto [--url <url>] [--exp <name> [--arg <value>] [--out <file>] [--screenshot <png>]]

  --url <url>         first page of the tab view (default https://example.com/)
  --exp <name>        run one experiment, print its JSON result and quit;
                      `--exp list` names them
  --arg <value>       input for the experiment (shell: the text to type)
  --out <file>        also write the result to <file>
  --screenshot <png>  capture the window when the experiment ends";

pub const WINDOW: &str = "main";
pub const UI_LABEL: &str = "ui-main";
pub const OVERLAY_LABEL: &str = "overlay-main";
const WINDOW_SIZE: (f64, f64) = (1200.0, 800.0);
/// Where experiment windows go, as agent tabs do (13 §5).
const OFF_SCREEN: (f64, f64) = (-20000.0, -20000.0);
/// Where the page starts until the UI reports its real content rectangle.
const INITIAL_PAGE_TOP: f64 = 44.0;
/// How long a scripted step waits before it reports a failure.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct Args {
    pub url: String,
    pub exp: Option<String>,
    pub arg: Option<String>,
    pub out: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
}

impl Args {
    pub fn parse(mut it: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut args = Args {
            url: "https://example.com/".into(),
            exp: None,
            arg: None,
            out: None,
            screenshot: None,
        };
        while let Some(flag) = it.next() {
            let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
            match flag.as_str() {
                "--url" => args.url = normalize_url(&value()?)?,
                "--exp" => args.exp = Some(value()?),
                "--arg" => args.arg = Some(value()?),
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
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Default)]
struct Reports {
    count: u64,
    last: Option<Rect>,
}

#[derive(Debug, Default)]
pub struct Shell {
    tab: OnceLock<platform::TabId>,
    reports: Mutex<Reports>,
    reported: Condvar,
}

impl Shell {
    pub fn tab(&self) -> Result<platform::TabId, String> {
        self.tab
            .get()
            .copied()
            .ok_or_else(|| "the tab view does not exist yet".into())
    }

    /// How many page rectangles the UI has reported, and the last one.
    pub fn reports(&self) -> Result<(u64, Option<Rect>), String> {
        let r = self.reports.lock().map_err(|_| "reports lock poisoned")?;
        Ok((r.count, r.last))
    }

    /// Waits until the UI has reported more than `seen` page rectangles.
    pub fn wait_report(&self, seen: u64, timeout: Duration) -> Result<(u64, Rect), String> {
        let deadline = Instant::now() + timeout;
        let mut r = self.reports.lock().map_err(|_| "reports lock poisoned")?;
        loop {
            if r.count > seen
                && let Some(last) = r.last
            {
                return Ok((r.count, last));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!(
                    "the UI reported no page rectangle within {timeout:?}"
                ));
            }
            r = self
                .reported
                .wait_timeout(r, left)
                .map_err(|_| "reports lock poisoned")?
                .0;
        }
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
    let mut reports = shell.reports.lock().map_err(|_| "reports lock poisoned")?;
    reports.count += 1;
    reports.last = Some(rect);
    shell.reported.notify_all();
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

/// What an experiment gets: the app, its one tab view and the CLI input.
#[derive(Debug)]
pub struct Ctx {
    pub app: AppHandle,
    pub tab: platform::TabId,
    pub arg: Option<String>,
}

impl Ctx {
    pub fn shell(&self) -> tauri::State<'_, Shell> {
        self.app.state::<Shell>()
    }

    pub fn window(&self) -> Result<tauri::Window, String> {
        self.app
            .get_window(WINDOW)
            .ok_or_else(|| "no main window".into())
    }
}

fn build_window(
    app: &mut tauri::App,
    url: &str,
    background: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    if background {
        app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    }
    let mut builder = WindowBuilder::new(app, WINDOW)
        .title("Mado M0 prototype")
        .inner_size(WINDOW_SIZE.0, WINDOW_SIZE.1);
    if background {
        // Experiments run while someone uses the Mac: no Dock icon, no
        // activation, no focus, and the window off-screen, ordered in but
        // never key (the agent-tab placement in 13 §5). An occluded window
        // would not do: WebKit pauses rendering there, and with it
        // requestAnimationFrame and ResizeObserver.
        builder = builder.position(OFF_SCREEN.0, OFF_SCREEN.1).focused(false);
    }
    let window = builder.build()?;
    if !background {
        window.set_focus()?;
    }
    // 1. The UI webview fills the window's content area.
    let size = window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?);
    window.add_child(
        WebviewBuilder::new(UI_LABEL, WebviewUrl::App("index.html".into())).auto_resize(),
        LogicalPosition::new(0.0, 0.0),
        size,
    )?;
    // 3. The overlay, hidden until something opens in it.
    let overlay = window.add_child(
        WebviewBuilder::new(OVERLAY_LABEL, WebviewUrl::App("overlay.html".into())),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(1.0, 1.0),
    )?;
    overlay.hide()?;
    platform::register_overlay(&window)?;
    // 2. Tab views go between them.
    let page = Rect {
        x: 0.0,
        y: INITIAL_PAGE_TOP,
        width: size.width,
        height: size.height - INITIAL_PAGE_TOP,
    };
    let tab = platform::create_tab(&window, page, url)?;
    app.state::<Shell>()
        .tab
        .set(tab)
        .map_err(|_| "tab created twice")?;
    Ok(())
}

pub fn run(args: Args) -> ExitCode {
    let Args {
        url,
        exp,
        arg,
        out,
        screenshot,
    } = args;
    if exp.as_deref() == Some("list") {
        println!("{}", crate::exp::NAMES.join("\n"));
        return ExitCode::SUCCESS;
    }
    if let Some(name) = &exp
        && !crate::exp::NAMES.contains(&name.as_str())
    {
        eprintln!("mado-proto: no experiment {name}; `--exp list` names them");
        return ExitCode::from(2);
    }
    let built = tauri::Builder::default()
        .manage(Shell::default())
        .invoke_handler(tauri::generate_handler![navigate, report_content_rect])
        .setup(move |app| {
            build_window(app, &url, exp.is_some())?;
            if let Some(name) = exp.clone() {
                let ctx = Ctx {
                    app: app.handle().clone(),
                    tab: app.state::<Shell>().tab()?,
                    arg: arg.clone(),
                };
                let (out, screenshot) = (out.clone(), screenshot.clone());
                std::thread::spawn(move || run_experiment(&ctx, &name, out, screenshot));
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

fn run_experiment(ctx: &Ctx, name: &str, out: Option<PathBuf>, screenshot: Option<PathBuf>) {
    let started = Instant::now();
    let result = ctx
        .shell()
        .wait_report(0, STEP_TIMEOUT)
        .and_then(|_| crate::exp::run(name, ctx));
    let shot = screenshot.map(|path| {
        platform::screenshot(&ctx.app, WINDOW, &path).unwrap_or_else(|e| format!("failed: {e}"))
    });
    let (code, result) = match result {
        Ok(value) => {
            let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(true);
            (i32::from(!ok), value)
        }
        Err(error) => {
            // The view stack usually explains a failed step.
            let views = ctx
                .window()
                .and_then(|window| on_main(&ctx.app, move || platform::describe_views(&window))?);
            let views = views.unwrap_or_else(|e| json!(e));
            (1, json!({ "ok": false, "error": error, "views": views }))
        }
    };
    let report = json!({
        "experiment": name,
        "result": result,
        "screenshot": shot,
        "seconds": started.elapsed().as_secs_f64(),
        "environment": platform::environment(),
    });
    let text = serde_json::to_string_pretty(&report).unwrap_or_else(|e| e.to_string());
    println!("{text}");
    if let Some(path) = out
        && let Err(e) = std::fs::write(&path, format!("{text}\n"))
    {
        eprintln!("mado-proto: cannot write {}: {e}", path.display());
    }
    ctx.app.exit(code);
}
