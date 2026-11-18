//! Subscription CLIs (spec 13 §2.3): finding them on the person's `PATH`,
//! their versions, and running the vendor's install and login commands in
//! the person's own terminal. Mado never reads the CLIs' token files.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use mado_lists::CliInfo;

/// What detection found for one CLI.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub provider: String,
    pub name: String,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    /// The ACP command's program, when it is found.
    pub acp: Option<PathBuf>,
    /// `npx` adapters need Node.js 18 or later.
    pub needs_node: bool,
    pub node_ok: bool,
    pub install: String,
    pub login: String,
    pub login_hint: Option<String>,
}

impl Found {
    /// Installed and its ACP command can start.
    pub fn ready(&self) -> bool {
        self.path.is_some() && self.acp.is_some() && (!self.needs_node || self.node_ok)
    }
}

/// The folders searched: the login shell's `PATH` (asked once) and the
/// usual install places.
pub fn search_dirs() -> Vec<PathBuf> {
    static LOGIN: OnceLock<Vec<PathBuf>> = OnceLock::new();
    let mut dirs = LOGIN
        .get_or_init(|| {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
            run(
                Path::new(&shell),
                &["-lc", "printf %s \"$PATH\""],
                Duration::from_secs(5),
            )
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default()
        })
        .clone();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".npm-global/bin"));
    }
    dirs.push("/opt/homebrew/bin".into());
    dirs.push("/usr/local/bin".into());
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p));
    }
    let mut seen = std::collections::BTreeSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

/// `name` in the first of `dirs` that has it, or `name` itself when it is
/// a path.
pub fn which(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    dirs.iter().map(|d| d.join(name)).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Runs `program args` with a time limit; its first output line.
fn run(program: &Path, args: &[&str], limit: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    use std::io::Read as _;
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out.trim().to_owned())
}

/// The major version `node --version` reports (`v20.1.0` is 20).
pub fn node_major(text: &str) -> Option<u32> {
    text.trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// Looks for one CLI.
pub fn detect(provider: &str, name: &str, info: &CliInfo, dirs: &[PathBuf]) -> Found {
    let path = which(&info.binary, dirs);
    let version = path
        .as_deref()
        .and_then(|p| run(p, &["--version"], Duration::from_secs(5)))
        .map(|v| v.lines().next().unwrap_or_default().to_owned());
    let program = info.acp.first().map(String::as_str).unwrap_or_default();
    let needs_node = program == "npx";
    let acp = which(program, dirs);
    let node_ok = !needs_node
        || which("node", dirs)
            .and_then(|n| run(&n, &["--version"], Duration::from_secs(5)))
            .and_then(|v| node_major(&v))
            .is_some_and(|m| m >= 18);
    Found {
        provider: provider.to_owned(),
        name: name.to_owned(),
        path,
        version,
        acp,
        needs_node,
        node_ok,
        install: info.install.clone(),
        login: info.login.clone(),
        login_hint: info.login_hint.clone(),
    }
}

/// The text of the `.command` file the terminal runs: the command, shown
/// first, then a pause so the person reads how it went.
pub fn command_file(command: &str, path: &str) -> String {
    format!(
        "#!/bin/sh\n# Mado runs this for you in your own terminal (spec 13 §2.3).\nexport PATH=\"{path}:$PATH\"\necho '$ {shown}'\n{command}\necho\necho 'Done. You can close this window and go back to Mado.'\n",
        shown = command.replace('\'', "'\\''"),
    )
}

/// Opens `command` in Terminal with a temporary `.command` file in `dir`.
pub fn open_in_terminal(command: &str, dir: &Path, dirs: &[PathBuf]) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(dir)?;
    let file = dir.join(format!("mado-{}.command", std::process::id()));
    let path = std::env::join_paths(dirs)
        .map_or_else(|_| String::new(), |p| p.to_string_lossy().into_owned());
    std::fs::write(&file, command_file(command, &path))?;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700))?;
    let status = Command::new("/usr/bin/open")
        .args(["-a", "Terminal"])
        .arg(&file)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("open -a Terminal failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_versions_parse() {
        assert_eq!(node_major("v20.11.1\n"), Some(20));
        assert_eq!(node_major("v16.0.0"), Some(16));
        assert_eq!(node_major("node"), None);
    }

    #[test]
    fn detection_finds_a_cli_and_its_adapter() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("dir");
        let make = |name: &str, out: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, format!("#!/bin/sh\necho '{out}'\n")).expect("write");
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        };
        make("claude", "2.1.0 (Claude Code)");
        make("npx", "");
        make("node", "v16.3.0");
        let info = mado_lists::models().cli["claudeCode"].clone();
        let dirs = vec![dir.path().to_owned()];
        let found = detect("claudeCode", "Claude Code", &info, &dirs);
        assert_eq!(found.version.as_deref(), Some("2.1.0 (Claude Code)"));
        assert!(
            found.needs_node && !found.node_ok && !found.ready(),
            "Node 16 is too old"
        );
        make("node", "v22.0.0");
        assert!(detect("claudeCode", "Claude Code", &info, &dirs).ready());
        let codex = mado_lists::models().cli["codex"].clone();
        assert!(detect("codex", "Codex", &codex, &dirs).path.is_none());
    }

    #[test]
    fn the_command_file_shows_the_command_first() {
        let f = command_file("npm install -g @openai/codex", "/opt/homebrew/bin");
        assert!(f.starts_with("#!/bin/sh\n"));
        assert!(
            f.contains("echo '$ npm install -g @openai/codex'\nnpm install -g @openai/codex\n")
        );
    }
}
