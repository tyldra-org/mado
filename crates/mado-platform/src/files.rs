//! Files for downloads (spec 12 §4, 08 §4): free space, the quarantine mark,
//! opening and revealing with the system, and the pasteboard.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

use objc2_app_kit::NSPasteboard;
use objc2_foundation::{NSArray, NSString, NSURL};

use crate::PlatformError;

fn c_path(path: &Path) -> Result<CString, PlatformError> {
    CString::new(path.as_os_str().as_bytes()).map_err(|e| PlatformError::Failed(e.to_string()))
}

/// Bytes free for this user on the volume holding `path`.
pub fn free_space(path: &Path) -> Result<u64, PlatformError> {
    let c = c_path(path)?;
    // SAFETY: statvfs fills the zeroed struct for a valid C path.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &raw mut stat) };
    if rc != 0 {
        return Err(PlatformError::Failed(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok(u64::from(stat.f_bavail).saturating_mul(stat.f_frsize))
}

/// `com.apple.quarantine` with Mado as the agent and the origin URL in
/// the quarantine database's sense (08 §4): Gatekeeper asks before a
/// downloaded app runs.
pub fn quarantine(path: &Path, now_secs: u64) -> Result<(), PlatformError> {
    // Flags 0081: downloaded, user approval needed. The UUID links to the
    // quarantine events database, which Mado does not write.
    let value = format!("0081;{now_secs:08x};Mado;");
    let c = c_path(path)?;
    let name = c"com.apple.quarantine";
    // SAFETY: valid C strings and a byte buffer of the given length.
    let rc = unsafe {
        libc::setxattr(
            c.as_ptr(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
            0,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(PlatformError::Failed(
            std::io::Error::last_os_error().to_string(),
        ))
    }
}

/// Opens a file with its default app.
pub fn open(path: &Path) -> Result<(), PlatformError> {
    run(Command::new("/usr/bin/open").arg("--").arg(path))
}

/// Opens a web page in Safari (site engine rules, spec 05 §9 and §11).
pub fn open_in_safari(url: &str) -> Result<(), PlatformError> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(PlatformError::Failed(
            "only web pages open in Safari".into(),
        ));
    }
    run(Command::new("/usr/bin/open").args(["-b", "com.apple.Safari", "--", url]))
}

/// Shows a file in Finder.
pub fn reveal(path: &Path) -> Result<(), PlatformError> {
    run(Command::new("/usr/bin/open").arg("-R").arg("--").arg(path))
}

fn run(cmd: &mut Command) -> Result<(), PlatformError> {
    let status = cmd
        .status()
        .map_err(|e| PlatformError::Failed(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::Failed(format!("open exited with {status}")))
    }
}

/// Puts the file itself on the pasteboard, as Finder's Copy does. Call on
/// the main thread.
pub fn copy_file(path: &Path) -> Result<(), PlatformError> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    let list = NSArray::from_retained_slice(&[objc2::rc::Retained::into_super(url)]);
    // SAFETY: NSURL conforms to NSPasteboardWriting.
    let ok: bool = unsafe { objc2::msg_send![&*board, writeObjects: &*list] };
    ok.then_some(())
        .ok_or_else(|| PlatformError::Failed("the pasteboard refused the file".into()))
}

/// The general pasteboard's change count and its text, for the copied-link
/// suggestion (spec 05 §2). Reads only; any thread.
pub fn pasteboard_text() -> (isize, Option<String>) {
    let board = NSPasteboard::generalPasteboard();
    // SAFETY: the string type constant is AppKit's.
    let text = board.stringForType(unsafe { objc2_app_kit::NSPasteboardTypeString });
    (board.changeCount(), text.map(|t| t.to_string()))
}

/// Puts text on the pasteboard. Call on the main thread.
pub fn copy_text(text: &str) -> Result<(), PlatformError> {
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    // SAFETY: the string type constant is AppKit's.
    let ok = board.setString_forType(&NSString::from_str(text), unsafe {
        objc2_app_kit::NSPasteboardTypeString
    });
    ok.then_some(())
        .ok_or_else(|| PlatformError::Failed("the pasteboard refused the text".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_and_the_quarantine_mark() {
        let dir = tempfile::tempdir().expect("dir");
        assert!(free_space(dir.path()).expect("statvfs") > 0);
        let f = dir.path().join("x.bin");
        std::fs::write(&f, b"x").expect("write");
        quarantine(&f, 1).expect("mark");
        let out = Command::new("/usr/bin/xattr")
            .arg("-p")
            .arg("com.apple.quarantine")
            .arg(&f)
            .output()
            .expect("xattr");
        assert!(String::from_utf8_lossy(&out.stdout).starts_with("0081;00000001;Mado;"));
    }
}
