//! What the running engine supports (spec 02 §2), so the core and UI hide a
//! feature instead of offering a broken one.

use serde::{Deserialize, Serialize};

/// Permission keys (spec 07 §1).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionKind {
    Camera,
    Microphone,
    Screen,
    Location,
    Notifications,
    ClipboardRead,
    Midi,
    MotionSensors,
    IdleDetection,
    LocalNetwork,
    WindowManagement,
    SpeechRecognition,
    Popups,
    AutomaticDownloads,
    Sound,
    Javascript,
    Images,
    InsecureContent,
    AutomaticFullscreen,
    ProtectedContent,
    /// `externalApp:<scheme>`.
    ExternalApp(String),
    ApplePay,
}

/// DRM key systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeySystem {
    FairPlay,
    Widevine,
    PlayReady,
}

/// Formats `Save page as` can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveFormat {
    /// `.webarchive` (WebKit).
    WebArchive,
    Pdf,
    HtmlOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub isolated_worlds: bool,
    /// Back and forward history survive sleep and restore.
    pub session_restore: bool,
    /// The engine counts find matches itself (no engine does; the `find`
    /// page script counts).
    pub find_match_count: bool,
    pub request_interception: bool,
    pub content_rule_lists: bool,
    /// A native `Notification` API Mado can drive (WebKit: shimmed).
    pub notifications_api: bool,
    pub web_push: bool,
    /// Permission keys the engine can enforce; the rest are hidden.
    pub permissions: Vec<PermissionKind>,
    pub extension_apis: Vec<String>,
    pub drm: Vec<KeySystem>,
    pub save_formats: Vec<ArchiveFormat>,
    pub apple_pay: bool,
}

impl Capabilities {
    /// WKWebView on macOS, as measured in M0 (spec 02 §2, 07 §1 and §9).
    /// Extension APIs are filled by the extension host (M3-T04).
    pub fn webkit() -> Self {
        use PermissionKind::*;
        Self {
            isolated_worlds: true,
            session_restore: true,
            find_match_count: false,
            request_interception: false,
            content_rule_lists: true,
            notifications_api: false,
            web_push: false,
            permissions: vec![
                Camera,
                Microphone,
                Screen,
                Location,
                Notifications,
                ClipboardRead,
                SpeechRecognition,
                Popups,
                AutomaticDownloads,
                Sound,
                Javascript,
                Images,
                InsecureContent,
                AutomaticFullscreen,
                ProtectedContent,
            ],
            extension_apis: Vec::new(),
            drm: vec![KeySystem::FairPlay],
            save_formats: vec![
                ArchiveFormat::WebArchive,
                ArchiveFormat::Pdf,
                ArchiveFormat::HtmlOnly,
            ],
            apple_pay: false,
        }
    }

    /// Whether the engine can enforce `kind`. App links are Mado's own.
    pub fn supports(&self, kind: &PermissionKind) -> bool {
        matches!(kind, PermissionKind::ExternalApp(_)) || self.permissions.contains(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webkit_hides_what_m0_found_missing() {
        let caps = Capabilities::webkit();
        assert!(!caps.apple_pay);
        assert!(!caps.notifications_api);
        assert!(!caps.find_match_count);
        assert!(!caps.supports(&PermissionKind::Midi));
        assert!(!caps.supports(&PermissionKind::ApplePay));
        assert!(caps.supports(&PermissionKind::Camera));
        assert!(caps.supports(&PermissionKind::ExternalApp("zoommtg".into())));
    }
}
