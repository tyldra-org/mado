//! Launch arguments (spec 01 §1.1 step 1).

use std::ffi::OsString;
use std::path::PathBuf;

use mado_types::command::LaunchSource;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    /// `--profile <ProfileId|name>`.
    pub profile: Option<String>,
    pub private: bool,
    pub guest: bool,
    /// No window; used by the CLI.
    pub hidden: bool,
    pub new_window: bool,
    pub urls: Vec<String>,
    pub files: Vec<PathBuf>,
    /// Internal: a profile picker handing a launch to another process.
    pub handoff_token: Option<String>,
}

impl Args {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut out = Self::default();
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            let text = arg.to_string_lossy().into_owned();
            let mut value = |name: &str| {
                it.next()
                    .map(|v| v.to_string_lossy().into_owned())
                    .ok_or_else(|| format!("{name} needs a value"))
            };
            match text.as_str() {
                "--profile" => out.profile = Some(value("--profile")?),
                "--handoff-token" => out.handoff_token = Some(value("--handoff-token")?),
                "--private" => out.private = true,
                "--guest" => out.guest = true,
                "--hidden" => out.hidden = true,
                "--new-window" => out.new_window = true,
                // macOS passes these to apps started from Finder.
                s if s.starts_with("-psn_") => {}
                s if s.starts_with("--") => return Err(format!("unknown option {s}")),
                s if looks_like_url(s) => out.urls.push(s.to_owned()),
                _ => out.files.push(PathBuf::from(arg)),
            }
        }
        Ok(out)
    }

    /// Where the launch came from, for `launch.forward`.
    pub fn source(&self) -> LaunchSource {
        if !self.urls.is_empty() {
            LaunchSource::Link
        } else if !self.files.is_empty() {
            LaunchSource::File
        } else if self.hidden {
            LaunchSource::Cli
        } else {
            LaunchSource::Dock
        }
    }
}

fn looks_like_url(s: &str) -> bool {
    s.split_once(':').is_some_and(|(scheme, rest)| {
        !rest.is_empty()
            && scheme.len() > 1
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn options_urls_and_files() {
        let a = parse(&[
            "--profile",
            "Work",
            "--new-window",
            "https://example.com/",
            "mailto:a@b.test",
            "/tmp/page.html",
            "-psn_0_123",
        ])
        .expect("parses");
        assert_eq!(a.profile.as_deref(), Some("Work"));
        assert!(a.new_window);
        assert_eq!(a.urls, ["https://example.com/", "mailto:a@b.test"]);
        assert_eq!(a.files, [PathBuf::from("/tmp/page.html")]);
        assert_eq!(a.source(), LaunchSource::Link);
        assert_eq!(parse(&[]).expect("parses").source(), LaunchSource::Dock);
        assert_eq!(
            parse(&["--hidden"]).expect("parses").source(),
            LaunchSource::Cli
        );
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(parse(&["--profile"]).is_err());
        assert!(parse(&["--nope"]).is_err());
    }
}
