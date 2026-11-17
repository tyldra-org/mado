//! Finding a running profile (spec 14 §1): `<root>/Control/<ProfileId>.json`
//! entries whose process lives and whose endpoint answers; stale ones are
//! deleted. Starting Mado hidden when none runs (14 §3).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub id: String,
    pub name: String,
    pub endpoint: PathBuf,
    pub pid: i32,
    pub started_at: i64,
}

/// Whether a process exists (`kill -0`, which sends nothing).
fn alive(pid: i32) -> bool {
    pid > 0
        && std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

/// The live profiles under `root`, newest first.
pub fn running(root: &Path) -> Vec<Running> {
    let dir = root.join("Control");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(v) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            continue;
        };
        let r = Running {
            id: v["profileId"].as_str().unwrap_or_default().to_owned(),
            name: v["name"].as_str().unwrap_or_default().to_owned(),
            endpoint: PathBuf::from(v["endpoint"].as_str().unwrap_or_default()),
            pid: v["pid"]
                .as_i64()
                .and_then(|p| i32::try_from(p).ok())
                .unwrap_or(0),
            started_at: v["startedAt"].as_i64().unwrap_or(0),
        };
        if alive(r.pid) && std::os::unix::net::UnixStream::connect(&r.endpoint).is_ok() {
            out.push(r);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.started_at));
    out
}

/// `wanted` by ID or name, else the newest.
pub fn pick(list: &[Running], wanted: Option<&str>) -> Option<Running> {
    match wanted {
        Some(w) => list
            .iter()
            .find(|r| r.id == w || r.name.eq_ignore_ascii_case(w))
            .cloned(),
        None => list.first().cloned(),
    }
}

/// Mado's executable: `MADO_APP`, else beside this one in the app bundle.
fn app() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MADO_APP") {
        return Some(PathBuf::from(p));
    }
    let me = std::env::current_exe().ok()?;
    let dir = me.parent()?;
    ["Mado", "mado-app"]
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
}

/// Starts Mado with `--hidden` and waits up to 20 seconds for the profile.
pub fn start(root: &Path, profile: Option<&str>) -> Result<Running, String> {
    let app = app().ok_or("Mado's app was not found next to mado; set MADO_APP")?;
    let mut cmd = std::process::Command::new(app);
    cmd.arg("--hidden");
    if let Some(p) = profile {
        cmd.args(["--profile", p]);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .map_err(|e| format!("Mado did not start: {e}"))?;
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if let Some(r) = pick(&running(root), profile) {
            return Ok(r);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err("Mado did not answer within 20 seconds".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_entries_are_deleted_and_live_ones_found() {
        let dir = tempfile::tempdir().expect("dir");
        let control = dir.path().join("Control");
        std::fs::create_dir_all(&control).expect("dir");
        let sock = dir.path().join("s.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind");
        let me = std::process::id();
        std::fs::write(
            control.join("a.json"),
            format!(
                r#"{{"profileId":"a","name":"Work","endpoint":{:?},"pid":{me},"startedAt":2}}"#,
                sock
            ),
        )
        .expect("write");
        std::fs::write(
            control.join("b.json"),
            r#"{"profileId":"b","name":"Old","endpoint":"/nonexistent.sock","pid":999999,"startedAt":3}"#,
        )
        .expect("write");
        let list = running(dir.path());
        assert_eq!(
            list.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a"]
        );
        assert!(!control.join("b.json").exists(), "the stale entry is gone");
        assert_eq!(pick(&list, Some("work")).map(|r| r.id), Some("a".into()));
        assert_eq!(pick(&list, Some("x")), None);
    }
}
