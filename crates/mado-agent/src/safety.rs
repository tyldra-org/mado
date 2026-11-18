//! Approvals and safety (spec 13 §9): the mode table, which actions are
//! consequential, totals read from a payment page, and the cross-site data
//! guard. Pure rules; the core asks the person.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

/// `ai.approvalMode` (13 §9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Ask,
    Auto,
    Full,
}

impl Mode {
    pub fn from_key(s: &str) -> Self {
        match s {
            "ask" => Self::Ask,
            "full" => Self::Full,
            _ => Self::Auto,
        }
    }
}

/// What an action does, for the card and for remembering a denial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Class {
    Pay,
    Transfer,
    Delete,
    Send,
    Confirm,
    Upload,
    SignIn,
    Settings,
    Install,
    AccountChange,
    /// Text read on another site, or a secret, typed into a page (9.5).
    CrossSiteData,
    LeaveScope,
    TypeInForm,
    Download,
    /// Acting in one of the person's own tabs (`ai.agentsInUserTabs` =
    /// `askFirst`, 13 §7.3).
    UserTab,
    /// A CLI agent's file change, or a file outside its task folder (13 §7.2).
    Files,
    /// A CLI agent's command on the Mac (13 §7.2).
    Terminal,
}

impl Class {
    pub fn key(self) -> &'static str {
        match self {
            Self::Pay => "pay",
            Self::Transfer => "transfer",
            Self::Delete => "delete",
            Self::Send => "send",
            Self::Confirm => "confirm",
            Self::Upload => "upload",
            Self::SignIn => "signIn",
            Self::Settings => "settings",
            Self::Install => "install",
            Self::AccountChange => "accountChange",
            Self::CrossSiteData => "crossSiteData",
            Self::LeaveScope => "leaveScope",
            Self::TypeInForm => "typeInForm",
            Self::Download => "download",
            Self::UserTab => "userTab",
            Self::Files => "files",
            Self::Terminal => "terminal",
        }
    }

    fn from_lexicon(class: &str) -> Option<Self> {
        Some(match class {
            "pay" => Self::Pay,
            "transfer" => Self::Transfer,
            "delete" => Self::Delete,
            "send" => Self::Send,
            "confirm" => Self::Confirm,
            _ => return None,
        })
    }

    /// "Allow for this task" is not offered for payments and deletes (9.3).
    pub fn may_allow_for_task(self) -> bool {
        !matches!(self, Self::Pay | Self::Transfer | Self::Delete)
    }
}

/// 9.1's table: whether `class` needs the person in `mode`.
pub fn needs_approval(mode: Mode, class: Class) -> bool {
    match class {
        Class::LeaveScope => mode != Mode::Full,
        Class::TypeInForm | Class::Download => mode == Mode::Ask,
        _ => true,
    }
}

/// What `ai-reader`'s `inspect` says about an element.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Element {
    pub name: String,
    pub value: String,
    pub nearby: String,
    pub submit: bool,
    pub in_form: bool,
    pub form_has_card: bool,
    pub form_has_password: bool,
    pub form_action_host: Option<String>,
    /// An email, password or phone field on a page about account settings.
    pub account_field: bool,
    /// The page's total, as written (`$1,234.50`).
    pub total: Option<String>,
}

/// 9.2 for a click or Enter on `el` on a page of `host`.
pub fn classify_press(el: &Element, sensitive_hosts: &[String]) -> Option<Class> {
    let lexicon = mado_lists::action_lexicon();
    for text in [&el.name, &el.value, &el.nearby] {
        if let Some(c) = lexicon.class_of(text).and_then(Class::from_lexicon) {
            return Some(c);
        }
    }
    if el.form_has_card {
        return Some(Class::Pay);
    }
    if el.form_has_password {
        return Some(Class::SignIn);
    }
    if el.submit
        && let Some(h) = &el.form_action_host
        && (mado_lists::payment_frames().allows(&format!("https://{h}"))
            || sensitive_hosts
                .iter()
                .any(|s| h == s || h.ends_with(&format!(".{s}"))))
    {
        return Some(Class::Pay);
    }
    None
}

/// The host part of a URL.
pub fn host(url: &str) -> Option<String> {
    crate::untrusted::host(url)
}

/// Whether `url` is in the task's scope (9.5): the hosts the prompt named
/// and the person approved, and their subdomains. `None` is no scope.
pub fn in_scope(scope: Option<&BTreeSet<String>>, url: &str) -> bool {
    let Some(scope) = scope else { return true };
    let Some(h) = host(url) else { return false };
    scope
        .iter()
        .any(|s| h == *s || h.ends_with(&format!(".{s}")))
}

/// Hosts named in a prompt, its scope at the start (9.5).
pub fn hosts_in(prompt: &str) -> BTreeSet<String> {
    prompt
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | ')' | '<' | '>' | ','))
        .filter_map(|w| {
            let w = w.trim_end_matches(['.', '!', '?', ':', ';']);
            let url = if w.contains("://") {
                w.to_owned()
            } else {
                format!("https://{w}")
            };
            let h = host(&url)?;
            (h.contains('.')
                && h.rsplit('.').next().is_some_and(|tld| {
                    tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
                }))
            .then(|| h.trim_start_matches("www.").to_owned())
        })
        .collect()
}

/// The cross-site data guard (9.5): word 8-grams of text read from each
/// origin.
#[derive(Debug, Clone, Default)]
pub struct Fingerprints {
    by_origin: BTreeMap<String, BTreeSet<u64>>,
}

const GRAM: usize = 8;

fn grams(text: &str) -> Vec<u64> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '@' && c != '.')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.len() < GRAM {
        return Vec::new();
    }
    words
        .windows(GRAM)
        .map(|w| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            w.hash(&mut h);
            h.finish()
        })
        .collect()
}

impl Fingerprints {
    pub fn note(&mut self, origin: &str, text: &str) {
        let set = self.by_origin.entry(origin.to_owned()).or_default();
        for g in grams(text) {
            if set.len() < 200_000 {
                set.insert(g);
            }
        }
    }

    /// The other origin `text` repeats eight words of, if any.
    pub fn crosses(&self, target_origin: &str, text: &str) -> Option<String> {
        let typed = grams(text);
        self.by_origin
            .iter()
            .filter(|(o, _)| o.as_str() != target_origin)
            .find(|(_, set)| typed.iter().any(|g| set.contains(g)))
            .map(|(o, _)| o.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_table() {
        for m in [Mode::Ask, Mode::Auto, Mode::Full] {
            assert!(needs_approval(m, Class::Pay));
            assert!(needs_approval(m, Class::Send));
        }
        assert!(needs_approval(Mode::Auto, Class::LeaveScope));
        assert!(!needs_approval(Mode::Full, Class::LeaveScope));
        assert!(needs_approval(Mode::Ask, Class::TypeInForm));
        assert!(!needs_approval(Mode::Auto, Class::TypeInForm));
        assert!(!needs_approval(Mode::Auto, Class::Download));
        assert!(!Class::Pay.may_allow_for_task() && Class::Send.may_allow_for_task());
    }

    #[test]
    fn presses_are_classified_by_words_and_forms() {
        let el = |name: &str| Element {
            name: name.into(),
            ..Element::default()
        };
        assert_eq!(classify_press(&el("Place order"), &[]), Some(Class::Pay));
        assert_eq!(classify_press(&el("Jetzt kaufen"), &[]), Some(Class::Pay));
        assert_eq!(classify_press(&el("注文を確定"), &[]), Some(Class::Pay));
        assert_eq!(
            classify_press(&el("Delete account"), &[]),
            Some(Class::Delete)
        );
        assert_eq!(classify_press(&el("Send"), &[]), Some(Class::Send));
        assert_eq!(classify_press(&el("Show more"), &[]), None);
        assert_eq!(
            classify_press(&el("Ordering guide"), &[]),
            None,
            "whole words only"
        );
        let card = Element {
            name: "Continue".into(),
            form_has_card: true,
            ..Element::default()
        };
        assert_eq!(classify_press(&card, &[]), Some(Class::Pay));
        let bank = Element {
            name: "Go".into(),
            submit: true,
            form_action_host: Some("secure.chase.com".into()),
            ..Element::default()
        };
        assert_eq!(
            classify_press(&bank, &["chase.com".into()]),
            Some(Class::Pay)
        );
    }

    #[test]
    fn scope_comes_from_the_prompt() {
        let s = hosts_in(
            "Find the cheapest flight on www.kayak.com and compare with https://expedia.com/flights, thanks.",
        );
        assert_eq!(
            s,
            BTreeSet::from(["kayak.com".to_owned(), "expedia.com".to_owned()])
        );
        assert!(in_scope(Some(&s), "https://www.kayak.com/x"));
        assert!(!in_scope(Some(&s), "https://evil.example/"));
        assert!(in_scope(None, "https://anything.example/"));
        assert!(hosts_in("just words. nothing else").is_empty());
    }

    #[test]
    fn text_read_on_one_site_typed_on_another_is_caught() {
        let mut f = Fingerprints::default();
        f.note(
            "https://mail.example",
            "Dear Ada your account number is 4421 and the password reset code is 99812 thanks",
        );
        assert_eq!(
            f.crosses(
                "https://evil.example",
                "here: your account number is 4421 and the password reset code is 99812"
            ),
            Some("https://mail.example".into())
        );
        assert_eq!(
            f.crosses(
                "https://mail.example",
                "your account number is 4421 and the password reset code"
            ),
            None
        );
        assert_eq!(f.crosses("https://evil.example", "hello there"), None);
    }
}
