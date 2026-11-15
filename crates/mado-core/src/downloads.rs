//! Download rules (spec 12 §4, 08 §4): file names, where a file goes, and
//! which files are dangerous. Pure functions; the core runs the flow.

use std::path::{Path, PathBuf};

/// The suffix of a file still being written.
pub const TEMP_SUFFIX: &str = ".madodownload";

/// Free space kept after a download (spec 12 §4.2 step 4).
pub const SPARE_BYTES: u64 = 100 << 20;

/// Spec 12 §4.2 step 3: no path separators, reserved or control
/// characters, no leading or trailing dots and spaces, at most 200 bytes.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == '.' || c == ' ');
    let mut out = if trimmed.is_empty() {
        "download".to_owned()
    } else {
        trimmed.to_owned()
    };
    if out.len() > 200 {
        let (stem, ext) = split(&out);
        let ext = ext.to_owned();
        let mut stem = stem.to_owned();
        while stem.len() + ext.len() > 200 {
            stem.pop();
        }
        out = format!("{stem}{ext}");
    }
    out
}

/// `("report", ".tar.gz")` for `report.tar.gz`, `("a", ".pdf")` for `a.pdf`.
fn split(name: &str) -> (&str, &str) {
    let lower = name.to_ascii_lowercase();
    for double in [".tar.gz", ".tar.bz2", ".tar.xz"] {
        if lower.ends_with(double) && name.len() > double.len() {
            return name.split_at(name.len() - double.len());
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    }
}

/// A name in `dir` that no file and no download in progress uses:
/// `name (1).ext`, `name (2).ext` on clash.
pub fn unique(dir: &Path, name: &str, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let (stem, ext) = split(name);
    let free = |p: &Path| !taken(p) && !taken(&with_suffix(p));
    let first = dir.join(name);
    if free(&first) {
        return first;
    }
    (1..10_000)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| free(p))
        .unwrap_or(first)
}

/// `file.zip` → `file.zip.madodownload`.
pub fn with_suffix(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(TEMP_SUFFIX);
    PathBuf::from(s)
}

/// The final path of a temporary one.
pub fn without_suffix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    PathBuf::from(s.strip_suffix(TEMP_SUFFIX).unwrap_or(&s).to_owned())
}

/// A name from the URL when the server gives none.
pub fn name_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back().map(str::to_owned))
        })
        .filter(|s| !s.is_empty())
        .map(|s| percent_decode(&s))
        .unwrap_or_else(|| "download".to_owned())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(h) = s.get(i + 1..i + 3)
            && let Ok(b) = u8::from_str_radix(h, 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Types that run code (spec 08 §4).
const DANGEROUS: &[&str] = &[
    "dmg", "pkg", "mpkg", "app", "exe", "msi", "bat", "cmd", "ps1", "scr", "js", "vbs", "jar",
    "command", "terminal", "workflow", "docm", "xlsm", "pptm", "dotm", "xltm",
];

/// Archives that may hold an app (`.app` in archives, 08 §4).
const ARCHIVES: &[&str] = &["zip", "tar", "gz", "tgz", "bz2", "xz", "7z", "rar"];

/// Whether a file type is dangerous by itself; archives count, since they
/// may hold an app.
pub fn dangerous_type(name: &str) -> bool {
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.contains('.') && (DANGEROUS.contains(&ext.as_str()) || ARCHIVES.contains(&ext.as_str()))
}

/// Types `downloads.openSafeTypes` = `safe` opens when done.
pub fn safe_to_open(name: &str) -> bool {
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "pdf"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "heic"
            | "txt"
            | "csv"
            | "mp3"
            | "m4a"
            | "mp4"
            | "mov"
    )
}

/// An insecure download: from an http URL, or from an http page (08 §4).
pub fn insecure(url: &str, page: Option<&str>) -> bool {
    let http = |u: &str| u.to_ascii_lowercase().starts_with("http:");
    let local = |u: &str| {
        url::Url::parse(u)
            .ok()
            .and_then(|u| {
                u.host_str()
                    .map(|h| h == "localhost" || h.ends_with(".localhost") || h == "127.0.0.1")
            })
            .unwrap_or(false)
    };
    (http(url) && !local(url)) || page.is_some_and(|p| http(p) && !local(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn names_are_cleaned() {
        assert_eq!(sanitize("../etc/pass\u{0}wd"), "etcpasswd");
        assert_eq!(sanitize("  .hidden.  "), "hidden");
        assert_eq!(sanitize("a:b*c?.pdf"), "abc.pdf");
        assert_eq!(sanitize(""), "download");
        let long = format!("{}.pdf", "x".repeat(300));
        let s = sanitize(&long);
        assert!(s.len() <= 200 && s.ends_with(".pdf"));
    }

    #[test]
    fn clashes_get_numbers_before_the_extension() {
        let dir = Path::new("/d");
        let taken: BTreeSet<PathBuf> = ["/d/a.tar.gz", "/d/a (1).tar.gz.madodownload", "/d/b"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let is = |p: &Path| taken.contains(p);
        assert_eq!(
            unique(dir, "a.tar.gz", &is),
            PathBuf::from("/d/a (2).tar.gz")
        );
        assert_eq!(unique(dir, "b", &is), PathBuf::from("/d/b (1)"));
        assert_eq!(unique(dir, "c.pdf", &is), PathBuf::from("/d/c.pdf"));
    }

    #[test]
    fn danger_and_insecurity() {
        assert!(dangerous_type("Setup.DMG"));
        assert!(dangerous_type("thing.zip"));
        assert!(!dangerous_type("paper.pdf"));
        assert!(insecure("http://a.example/x.zip", None));
        assert!(insecure(
            "https://a.example/x.zip",
            Some("http://b.example/")
        ));
        assert!(!insecure(
            "https://a.example/x.zip",
            Some("https://b.example/")
        ));
        assert!(!insecure("http://localhost:8000/x", None));
        assert_eq!(
            name_from_url("https://a.example/files/My%20Report.pdf?x=1"),
            "My Report.pdf"
        );
        assert_eq!(
            without_suffix(&with_suffix(Path::new("/d/a.zip"))),
            PathBuf::from("/d/a.zip")
        );
    }
}
