//! App links (spec 07 §6): schemes never launched, and the launch flood
//! limit.

use std::collections::HashMap;

/// Schemes never handed to another app: 07 §6's list and Chromium's
/// `kDeniedSchemes` (`afp`, `data`, `disk`, `disks`, `file`, `hcp`,
/// `ie.http`, `javascript`, `ms-help`, `nntp`, `res`, `shell`,
/// `vbscript`, `view-source`, `vnd.ms.radio`).
const DENIED: &[&str] = &[
    "afp",
    "data",
    "disk",
    "disks",
    "file",
    "hcp",
    "ie.http",
    "javascript",
    "ms-help",
    "nntp",
    "res",
    "shell",
    "vbscript",
    "view-source",
    "vnd.ms.radio",
    "ms-msdt",
    "search-ms",
    "search",
    "ms-officecmd",
    "ms-cxh",
    "ms-cxh-full",
];

/// 07 §6 step 2: whether `url` may never open another app. Office schemes
/// are refused with their `ofe`/`ofv` commands; `ms-settings` without a
/// user gesture.
pub fn blocked_scheme(url: &str, user_gesture: bool) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return true;
    };
    let scheme = scheme.to_ascii_lowercase();
    if DENIED.contains(&scheme.as_str()) {
        return true;
    }
    if matches!(scheme.as_str(), "ms-word" | "ms-excel" | "ms-powerpoint") {
        let rest = rest.to_ascii_lowercase();
        return rest.starts_with("ofe|") || rest.starts_with("ofv|");
    }
    scheme == "ms-settings" && !user_gesture
}

/// 07 §6 step 4: more than 3 launches from one tab within 10 seconds are
/// refused.
#[derive(Debug, Default)]
pub struct LaunchLimiter<K> {
    launches: HashMap<K, Vec<i64>>,
}

impl<K: std::hash::Hash + Eq> LaunchLimiter<K> {
    pub fn new() -> Self {
        Self {
            launches: HashMap::new(),
        }
    }

    /// Counts a launch from `tab`; false when it is one too many.
    pub fn allow(&mut self, tab: K, now: i64) -> bool {
        let list = self.launches.entry(tab).or_default();
        list.retain(|at| now - at < 10_000);
        if list.len() >= 3 {
            return false;
        }
        list.push(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_schemes_never_launch() {
        assert!(blocked_scheme("javascript:alert(1)", true));
        assert!(blocked_scheme("search-ms:query=x", true));
        assert!(blocked_scheme("ms-word:ofe|u|https://x/y.doc", true));
        assert!(!blocked_scheme(
            "ms-word:ofv|u|https://x/y.doc"
                .replace("ofv", "nft")
                .as_str(),
            true
        ));
        assert!(blocked_scheme("ms-settings:privacy", false));
        assert!(!blocked_scheme("ms-settings:privacy", true));
        assert!(!blocked_scheme("zoommtg://zoom.us/join", false));
        assert!(!blocked_scheme("mailto:a@b.example", false));
    }

    #[test]
    fn a_fourth_launch_in_ten_seconds_is_refused() {
        let mut l = LaunchLimiter::new();
        assert!(l.allow(1, 0) && l.allow(1, 1000) && l.allow(1, 2000));
        assert!(!l.allow(1, 3000));
        assert!(l.allow(2, 3000));
        assert!(l.allow(1, 11_000));
    }
}
