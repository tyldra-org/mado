//! Running a password manager's CLI (spec 11 §6.2, §6.3): arguments as
//! separate argv entries (never a shell), secrets only in that child's
//! environment, and a time limit. Tests swap in a fake.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What a finished run gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

pub trait Runner: Send + Sync + std::fmt::Debug {
    /// Runs `program` with `args` and extra `env`, writing `stdin` when
    /// given. `None` when it could not start or ran past `limit`.
    fn run(
        &self,
        program: &Path,
        args: &[&str],
        env: &[(&str, &str)],
        stdin: Option<&str>,
        limit: Duration,
    ) -> Option<Output>;
}

/// Finds a CLI on `PATH` and where Homebrew installs it.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// Real processes.
#[derive(Debug, Default)]
pub struct System;

impl Runner for System {
    fn run(
        &self,
        program: &Path,
        args: &[&str],
        env: &[(&str, &str)],
        stdin: Option<&str>,
        limit: Duration,
    ) -> Option<Output> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().ok()?;
        if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
            use std::io::Write as _;
            let _ = pipe.write_all(text.as_bytes());
        }
        let mut out = child.stdout.take()?;
        let mut err = child.stderr.take()?;
        let reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = out.read_to_string(&mut s);
            s
        });
        let err_reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = err.read_to_string(&mut s);
            s
        });
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if start.elapsed() < limit => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        };
        Some(Output {
            ok: status.success(),
            stdout: reader.join().unwrap_or_default(),
            stderr: err_reader.join().unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_runs_with_its_env_and_a_time_limit() {
        let sh = Path::new("/bin/sh");
        let out = System
            .run(
                sh,
                &["-c", "printf %s \"$MADO_X\"; cat"],
                &[("MADO_X", "a b")],
                Some("in"),
                Duration::from_secs(5),
            )
            .expect("ran");
        assert!(out.ok);
        assert_eq!(out.stdout, "a bin");
        assert!(
            System
                .run(
                    sh,
                    &["-c", "sleep 5"],
                    &[],
                    None,
                    Duration::from_millis(200)
                )
                .is_none()
        );
        assert!(
            !System
                .run(sh, &["-c", "exit 3"], &[], None, Duration::from_secs(5))
                .expect("ran")
                .ok
        );
    }
}
