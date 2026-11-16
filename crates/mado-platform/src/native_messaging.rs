//! Native messaging hosts (spec 09 §9): finding a host's manifest, and
//! running it with Chrome's framing — a 4-byte length in native byte order,
//! then UTF-8 JSON.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

/// At most this many bytes from a host in one message.
pub const MAX_FROM_HOST: usize = 1024 * 1024;
/// At most this many bytes to a host in one message.
pub const MAX_TO_HOST: usize = 64 * 1024 * 1024;
/// At most this many host processes per extension at once.
pub const MAX_PER_EXTENSION: usize = 8;
/// A closed port's host is killed after this long.
const GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    #[error("no native messaging host {0}")]
    NotFound(String),
    #[error("the host's manifest is invalid: {0}")]
    Manifest(String),
    #[error("the host does not allow this extension")]
    NotAllowed,
    #[error("too many native hosts running for this extension")]
    TooMany,
    #[error("the message is too large")]
    TooLarge,
    #[error("the host exited")]
    Exited,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("the host sent invalid JSON")]
    Json,
}

/// A host found and checked for `extension`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub name: String,
    pub path: PathBuf,
}

/// Where Mado looks, first match wins (09 §9.1): Mado's own folder in the
/// data root, then Chrome's user and system folders, then Chromium, Edge,
/// Brave and Arc.
pub fn search_dirs(mado_root: &Path, home: &Path) -> Vec<PathBuf> {
    let support = home.join("Library/Application Support");
    vec![
        mado_root.join("NativeMessagingHosts"),
        support.join("Google/Chrome/NativeMessagingHosts"),
        PathBuf::from("/Library/Google/Chrome/NativeMessagingHosts"),
        support.join("Chromium/NativeMessagingHosts"),
        support.join("Microsoft Edge/NativeMessagingHosts"),
        support.join("BraveSoftware/Brave-Browser/NativeMessagingHosts"),
        support.join("Arc/User Data/NativeMessagingHosts"),
    ]
}

/// A host name: lowercase letters, digits, `_` and inner dots.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
}

/// Finds and checks host `name` for extension `extension_id`.
pub fn find(dirs: &[PathBuf], name: &str, extension_id: &str) -> Result<Host, NativeError> {
    if !valid_name(name) {
        return Err(NativeError::NotFound(name.to_owned()));
    }
    let Some(text) = dirs
        .iter()
        .find_map(|d| std::fs::read_to_string(d.join(format!("{name}.json"))).ok())
    else {
        return Err(NativeError::NotFound(name.to_owned()));
    };
    let m: Value = serde_json::from_str(&text).map_err(|e| NativeError::Manifest(e.to_string()))?;
    if m["type"] != "stdio" {
        return Err(NativeError::Manifest("type is not stdio".into()));
    }
    let path = m["path"].as_str().map(PathBuf::from).unwrap_or_default();
    if !path.is_absolute() {
        return Err(NativeError::Manifest("path is not absolute".into()));
    }
    let origin = format!("chrome-extension://{extension_id}/");
    let allowed = m["allowed_origins"]
        .as_array()
        .is_some_and(|a| a.iter().any(|o| o.as_str() == Some(origin.as_str())));
    if !allowed {
        return Err(NativeError::NotAllowed);
    }
    Ok(Host {
        name: name.to_owned(),
        path,
    })
}

/// One framed message.
pub fn write_message(out: &mut impl Write, message: &Value) -> Result<(), NativeError> {
    let bytes = serde_json::to_vec(message).map_err(|_| NativeError::Json)?;
    if bytes.len() > MAX_TO_HOST {
        return Err(NativeError::TooLarge);
    }
    let len = u32::try_from(bytes.len()).map_err(|_| NativeError::TooLarge)?;
    out.write_all(&len.to_ne_bytes())?;
    out.write_all(&bytes)?;
    out.flush()?;
    Ok(())
}

/// Reads one framed message; `None` at the end of the stream.
pub fn read_message(input: &mut impl Read) -> Result<Option<Value>, NativeError> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = usize::try_from(u32::from_ne_bytes(len)).map_err(|_| NativeError::TooLarge)?;
    if len > MAX_FROM_HOST {
        return Err(NativeError::TooLarge);
    }
    let mut body = vec![0u8; len];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|_| NativeError::Json)
}

/// Counts running hosts per extension.
#[derive(Debug, Default, Clone)]
pub struct Limits(Arc<Mutex<std::collections::HashMap<String, Arc<AtomicUsize>>>>);

/// A running host's slot; freed when dropped.
#[derive(Debug)]
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Limits {
    fn take(&self, extension: &str) -> Result<Slot, NativeError> {
        let counter = Arc::clone(
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entry(extension.to_owned())
                .or_default(),
        );
        if counter.fetch_add(1, Ordering::SeqCst) >= MAX_PER_EXTENSION {
            counter.fetch_sub(1, Ordering::SeqCst);
            return Err(NativeError::TooMany);
        }
        Ok(Slot(counter))
    }
}

fn spawn(host: &Host, extension_id: &str) -> Result<Child, NativeError> {
    let mut command = Command::new(&host.path);
    command
        .arg(format!("chrome-extension://{extension_id}/"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = host.path.parent() {
        command.current_dir(dir);
    }
    let mut child = command.spawn()?;
    // Its stderr goes to the log, a line at a time, 4 KB at most.
    if let Some(err) = child.stderr.take() {
        let name = host.name.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                let line: String = line.chars().take(4096).collect();
                tracing::info!(host = %name, "{line}");
            }
        });
    }
    Ok(child)
}

fn finish(mut child: Child) {
    drop(child.stdin.take());
    std::thread::spawn(move || {
        std::thread::sleep(GRACE);
        if matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
        }
        let _ = child.wait();
    });
}

/// `runtime.sendNativeMessage`: starts the host, sends one message, reads
/// one reply, and closes it. Blocks.
pub fn send_once(
    limits: &Limits,
    host: &Host,
    extension_id: &str,
    message: &Value,
) -> Result<Value, NativeError> {
    let _slot = limits.take(extension_id)?;
    let mut child = spawn(host, extension_id)?;
    let result = (|| {
        let stdin = child.stdin.as_mut().ok_or(NativeError::Exited)?;
        write_message(stdin, message)?;
        let stdout = child.stdout.as_mut().ok_or(NativeError::Exited)?;
        read_message(stdout)?.ok_or(NativeError::Exited)
    })();
    finish(child);
    result
}

/// `runtime.connectNative`: a host running for one port.
#[derive(Debug)]
pub struct Port {
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    _slot: Slot,
}

impl Port {
    /// Starts the host; `on_message` gets each message it sends, then
    /// `None` when it exits or breaks the framing. Both run on a reader
    /// thread.
    pub fn open(
        limits: &Limits,
        host: &Host,
        extension_id: &str,
        on_message: impl Fn(Option<Value>) + Send + 'static,
    ) -> Result<Self, NativeError> {
        let slot = limits.take(extension_id)?;
        let mut child = spawn(host, extension_id)?;
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().ok_or(NativeError::Exited)?;
        std::thread::spawn(move || {
            while let Ok(Some(m)) = read_message(&mut stdout) {
                on_message(Some(m));
            }
            on_message(None);
        });
        Ok(Self {
            stdin: Mutex::new(stdin),
            child: Mutex::new(Some(child)),
            _slot: slot,
        })
    }

    pub fn send(&self, message: &Value) -> Result<(), NativeError> {
        let mut stdin = self
            .stdin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stdin = stdin.as_mut().ok_or(NativeError::Exited)?;
        write_message(stdin, message)
    }

    /// Closes stdin; the host is killed if it is still running after 2 s.
    pub fn close(&self) {
        drop(
            self.stdin
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
        if let Some(child) = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            finish(child);
        }
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host that answers each message with `{"echo": <message>, "origin": <arg>}`.
    fn echo_host(dir: &Path) -> PathBuf {
        let path = dir.join("echo.py");
        std::fs::write(
            &path,
            r#"#!/usr/bin/env python3
import json, struct, sys
origin = sys.argv[1]
while True:
    raw = sys.stdin.buffer.read(4)
    if len(raw) < 4:
        break
    n = struct.unpack("=I", raw)[0]
    msg = json.loads(sys.stdin.buffer.read(n))
    out = json.dumps({"echo": msg, "origin": origin}).encode()
    sys.stdout.buffer.write(struct.pack("=I", len(out)) + out)
    sys.stdout.buffer.flush()
"#,
        )
        .expect("host");
        let mut perms = std::fs::metadata(&path).expect("meta").permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
        path
    }

    const EXT: &str = "abcdefghijklmnopabcdefghijklmnop";

    fn manifest(dir: &Path, name: &str, path: &Path, origin: &str) {
        std::fs::create_dir_all(dir).expect("dir");
        std::fs::write(
            dir.join(format!("{name}.json")),
            serde_json::json!({ "name": name, "type": "stdio", "path": path, "allowed_origins": [origin] }).to_string(),
        )
        .expect("manifest");
    }

    #[test]
    fn hosts_are_found_first_match_first_and_checked() {
        let tmp = tempfile::tempdir().expect("dir");
        let host = echo_host(tmp.path());
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        manifest(
            &a,
            "com.example.echo",
            &host,
            &format!("chrome-extension://{EXT}/"),
        );
        manifest(
            &b,
            "com.example.echo",
            Path::new("/elsewhere"),
            &format!("chrome-extension://{EXT}/"),
        );
        let dirs = [a.clone(), b];
        assert_eq!(
            find(&dirs, "com.example.echo", EXT).expect("found").path,
            host
        );
        assert!(matches!(
            find(
                &dirs,
                "com.example.echo",
                "pppppppppppppppppppppppppppppppp"
            ),
            Err(NativeError::NotAllowed)
        ));
        assert!(matches!(
            find(&dirs, "com.example.none", EXT),
            Err(NativeError::NotFound(_))
        ));
        assert!(matches!(
            find(&dirs, "../etc/passwd", EXT),
            Err(NativeError::NotFound(_))
        ));
        manifest(
            &a,
            "com.example.relative",
            Path::new("echo.py"),
            &format!("chrome-extension://{EXT}/"),
        );
        assert!(matches!(
            find(&dirs, "com.example.relative", EXT),
            Err(NativeError::Manifest(_))
        ));
    }

    #[test]
    fn a_message_and_a_port_reach_the_host_and_come_back() {
        let tmp = tempfile::tempdir().expect("dir");
        let host = Host {
            name: "echo".into(),
            path: echo_host(tmp.path()),
        };
        let limits = Limits::default();
        let reply =
            send_once(&limits, &host, EXT, &serde_json::json!({ "ping": 1 })).expect("reply");
        assert_eq!(reply["echo"]["ping"], 1);
        assert_eq!(reply["origin"], format!("chrome-extension://{EXT}/"));

        let (tx, rx) = std::sync::mpsc::channel();
        let port = Port::open(&limits, &host, EXT, move |m| {
            let _ = tx.send(m);
        })
        .expect("port");
        port.send(&serde_json::json!("a")).expect("a");
        port.send(&serde_json::json!("b")).expect("b");
        let got: Vec<Value> = (0..2)
            .map(|_| {
                rx.recv_timeout(Duration::from_secs(10))
                    .expect("message")
                    .expect("some")
            })
            .collect();
        assert_eq!(
            (got[0]["echo"].clone(), got[1]["echo"].clone()),
            (serde_json::json!("a"), serde_json::json!("b"))
        );
        port.close();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).expect("end"), None);
    }

    #[test]
    fn framing_is_bounded_and_counted() {
        let mut buf = Vec::new();
        write_message(&mut buf, &serde_json::json!({"a": [1, 2]})).expect("write");
        assert_eq!(&buf[..4], &(11u32).to_ne_bytes());
        assert_eq!(
            read_message(&mut buf.as_slice()).expect("read"),
            Some(serde_json::json!({"a": [1, 2]}))
        );
        let mut big = (u32::try_from(MAX_FROM_HOST).expect("fits") + 1)
            .to_ne_bytes()
            .to_vec();
        big.extend_from_slice(b"{}");
        assert!(matches!(
            read_message(&mut big.as_slice()),
            Err(NativeError::TooLarge)
        ));
        assert_eq!(read_message(&mut [].as_slice()).expect("eof"), None);
        let limits = Limits::default();
        let slots: Vec<Slot> = (0..MAX_PER_EXTENSION)
            .map(|_| limits.take(EXT).expect("slot"))
            .collect();
        assert!(matches!(limits.take(EXT), Err(NativeError::TooMany)));
        drop(slots);
        assert!(limits.take(EXT).is_ok());
    }
}
