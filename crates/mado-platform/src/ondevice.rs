//! Apple's on-device model (spec 13 §2.1 `onDevice`, §6) through the
//! `mado-ondevice` helper (`swift/ondevice.swift`): next to Mado's
//! executable in the app bundle, or where the build put it in development.
//! Calls block; run them on a blocking thread.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Whether the model can answer, or why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Available,
    /// Before macOS 26, or no helper.
    NotSupported,
    DeviceNotEligible,
    AppleIntelligenceOff,
    ModelNotReady,
    Unavailable,
}

impl Status {
    pub fn key(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::NotSupported => "notSupported",
            Self::DeviceNotEligible => "deviceNotEligible",
            Self::AppleIntelligenceOff => "appleIntelligenceOff",
            Self::ModelNotReady => "modelNotReady",
            Self::Unavailable => "unavailable",
        }
    }
}

/// macOS's major version, from `kern.osproductversion`.
fn macos_major() -> u32 {
    let mut buf = [0u8; 32];
    let mut len = buf.len();
    // SAFETY: a NUL-terminated name, a buffer and its length, as sysctlbyname
    // expects; it writes at most `len` bytes.
    let rc = unsafe {
        libc::sysctlbyname(
            c"kern.osproductversion".as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return 0;
    }
    String::from_utf8_lossy(&buf[..len.saturating_sub(1)])
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(0)
}

/// The helper, if this Mac can run it.
pub fn helper() -> Option<PathBuf> {
    if macos_major() < 26 {
        return None;
    }
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("mado-ondevice")));
    bundled
        .filter(|p| p.is_file())
        .or_else(|| Some(PathBuf::from(env!("MADO_ONDEVICE_DEV"))).filter(|p| p.is_file()))
}

pub fn status() -> Status {
    let Some(helper) = helper() else {
        return Status::NotSupported;
    };
    let Ok(out) = Command::new(helper).arg("status").output() else {
        return Status::Unavailable;
    };
    match String::from_utf8_lossy(&out.stdout).trim() {
        "available" => Status::Available,
        "deviceNotEligible" => Status::DeviceNotEligible,
        "appleIntelligenceOff" => Status::AppleIntelligenceOff,
        "modelNotReady" => Status::ModelNotReady,
        _ => Status::Unavailable,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OnDeviceError {
    #[error("Apple's on-device model is not available")]
    Unavailable,
    #[error("the on-device model did not answer")]
    Failed,
}

/// One answer to `prompt` under `instructions`. Blocks.
pub fn respond(instructions: &str, prompt: &str) -> Result<String, OnDeviceError> {
    let helper = helper().ok_or(OnDeviceError::Unavailable)?;
    let mut child = Command::new(helper)
        .arg("respond")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| OnDeviceError::Unavailable)?;
    let request = serde_json::json!({ "instructions": instructions, "prompt": prompt });
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(request.to_string().as_bytes())
            .map_err(|_| OnDeviceError::Failed)?;
    }
    let out = child
        .wait_with_output()
        .map_err(|_| OnDeviceError::Failed)?;
    let answer: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|_| OnDeviceError::Failed)?;
    match (answer["text"].as_str(), answer["error"].as_str()) {
        (Some(t), _) => Ok(t.to_owned()),
        (_, Some("unavailable")) => Err(OnDeviceError::Unavailable),
        _ => Err(OnDeviceError::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On this Mac the status is known; when the model is available it
    /// answers (M0: 0.2 to 2 s once loaded).
    #[test]
    fn the_on_device_model_answers_when_available() {
        let s = status();
        if s != Status::Available {
            assert!(respond("x", "y").is_err(), "{s:?}");
            return;
        }
        let answer =
            respond("Answer with one word.", "Reply with the word: ready").expect("an answer");
        assert!(!answer.trim().is_empty());
    }
}
