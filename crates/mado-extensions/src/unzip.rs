//! Unpacking a verified package (spec 09 §2.2 step 4): no entry may leave
//! the folder or be a link, and the count and the bytes actually written
//! are capped, whatever the archive's own sizes claim.

use std::io::{Read, Write};
use std::path::Path;

/// At most this many entries.
pub const MAX_FILES: usize = 5_000;
/// At most this many bytes once unpacked.
pub const MAX_BYTES: u64 = 500 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unpacked {
    pub files: usize,
    pub bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum UnzipError {
    #[error("the archive is malformed: {0}")]
    Archive(String),
    #[error("{0:?} would be written outside the folder")]
    Escapes(String),
    #[error("{0:?} is a link")]
    Link(String),
    #[error("over {MAX_FILES} files")]
    TooManyFiles,
    #[error("over {MAX_BYTES} bytes unpacked")]
    TooLarge,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Unpacks `zip` into `dest`, which must not exist yet.
pub fn unzip(zip: &[u8], dest: &Path) -> Result<Unpacked, UnzipError> {
    unzip_with(zip, dest, MAX_FILES, MAX_BYTES)
}

fn unzip_with(
    zip: &[u8],
    dest: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<Unpacked, UnzipError> {
    let archive_error = |e: zip::result::ZipError| UnzipError::Archive(e.to_string());
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(archive_error)?;
    if archive.len() > max_files {
        return Err(UnzipError::TooManyFiles);
    }
    std::fs::create_dir(dest)?;
    let mut total: u64 = 0;
    let mut files = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(archive_error)?;
        let name = String::from_utf8_lossy(entry.name_raw()).into_owned();
        if entry.is_symlink() {
            return Err(UnzipError::Link(name));
        }
        let parts: Vec<&str> = name.trim_end_matches('/').split('/').collect();
        if name.starts_with('/')
            || name.contains(['\\', ':', '\0'])
            || parts.iter().any(|p| *p == ".." || p.is_empty())
        {
            return Err(UnzipError::Escapes(name));
        }
        let Some(relative) = entry.enclosed_name() else {
            return Err(UnzipError::Escapes(name));
        };
        let path = dest.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let left = max_bytes - total;
        let written = std::io::copy(&mut (&mut entry).take(left + 1), &mut out)?;
        out.flush()?;
        if written > left {
            return Err(UnzipError::TooLarge);
        }
        total += written;
        files += 1;
    }
    Ok(Unpacked {
        files,
        bytes: total,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A ZIP of `(name, contents)`.
    pub(crate) fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            w.start_file(*name, options).expect("start");
            w.write_all(body).expect("write");
        }
        w.finish().expect("finish").into_inner()
    }

    #[test]
    fn a_package_unpacks_and_cannot_escape_or_explode() {
        let dir = tempfile::tempdir().expect("dir");
        let ok = zip_of(&[("manifest.json", b"{}"), ("js/a.js", b"1")]);
        let done = unzip(&ok, &dir.path().join("ok")).expect("unpacks");
        assert_eq!(done, Unpacked { files: 2, bytes: 3 });
        assert_eq!(
            std::fs::read(dir.path().join("ok/js/a.js")).expect("file"),
            b"1"
        );

        for (i, name) in ["../evil.js", "/etc/evil", "a/../../evil"]
            .iter()
            .enumerate()
        {
            let bad = zip_of(&[(name, b"x")]);
            assert!(
                matches!(
                    unzip(&bad, &dir.path().join(format!("bad{i}"))),
                    Err(UnzipError::Escapes(_))
                ),
                "{name}"
            );
        }
        assert!(!dir.path().join("evil.js").exists());

        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        w.add_symlink(
            "link",
            "/etc/passwd",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("link");
        let link = w.finish().expect("finish").into_inner();
        assert!(matches!(
            unzip(&link, &dir.path().join("link")),
            Err(UnzipError::Link(_))
        ));

        let zeros = vec![0u8; 4096];
        let bomb = zip_of(&[("a", &zeros), ("b", &zeros)]);
        assert!(matches!(
            unzip_with(&bomb, &dir.path().join("bomb"), 10, 5000),
            Err(UnzipError::TooLarge)
        ));
        assert!(matches!(
            unzip_with(&ok, &dir.path().join("many"), 1, MAX_BYTES),
            Err(UnzipError::TooManyFiles)
        ));
        assert!(unzip(b"not a zip", &dir.path().join("junk")).is_err());
    }
}
