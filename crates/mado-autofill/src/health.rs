//! Password health (spec 11 §9): reused, weak and leaked logins. Leak
//! checks send only the first five hex characters of each password's SHA-1
//! (Have I Been Pwned's range API, padded) and compare the rest here.

use std::collections::{BTreeMap, BTreeSet};

use sha1::{Digest as _, Sha1};

/// One login as health reads it.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub site: String,
    pub password: String,
}

/// What health found, by item ID.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Passwords used on two or more sites: the IDs sharing each.
    pub reused: Vec<Vec<String>>,
    /// zxcvbn score under 3.
    pub weak: BTreeSet<String>,
    /// Found in a breach; `None` when not checked.
    pub leaked: Option<BTreeSet<String>>,
}

pub fn is_weak(password: &str, inputs: &[&str]) -> bool {
    u8::from(zxcvbn::zxcvbn(password, inputs).score()) < 3
}

/// Reused and weak (no network).
pub fn local(entries: &[Entry]) -> Report {
    let mut by_password: BTreeMap<&str, (BTreeSet<&str>, Vec<String>)> = BTreeMap::new();
    for e in entries.iter().filter(|e| !e.password.is_empty()) {
        let slot = by_password.entry(e.password.as_str()).or_default();
        slot.0.insert(e.site.as_str());
        slot.1.push(e.id.clone());
    }
    let reused = by_password
        .into_values()
        .filter(|(sites, _)| sites.len() >= 2)
        .map(|(_, ids)| ids)
        .collect();
    let weak = entries
        .iter()
        .filter(|e| !e.password.is_empty() && is_weak(&e.password, &[e.site.as_str()]))
        .map(|e| e.id.clone())
        .collect();
    Report {
        reused,
        weak,
        leaked: None,
    }
}

/// A password's SHA-1, upper-case hex: the 5-character prefix and the rest.
pub fn hash(password: &str) -> (String, String) {
    let hex: String = Sha1::digest(password.as_bytes())
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();
    let (a, b) = hex.split_at(5);
    (a.to_owned(), b.to_owned())
}

/// Whether `suffix` is in a range answer (`SUFFIX:COUNT` lines; padding
/// lines have a count of 0).
pub fn in_range(answer: &str, suffix: &str) -> bool {
    answer.lines().any(|line| {
        line.split_once(':').is_some_and(|(s, n)| {
            s.trim().eq_ignore_ascii_case(suffix) && n.trim().parse::<u64>().is_ok_and(|n| n > 0)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, site: &str, pw: &str) -> Entry {
        Entry {
            id: id.into(),
            site: site.into(),
            password: pw.into(),
        }
    }

    #[test]
    fn reused_and_weak_passwords_are_flagged() {
        let r = local(&[
            e("a", "example.com", "correct horse battery staple 42!"),
            e("b", "other.org", "correct horse battery staple 42!"),
            e("c", "example.com", "password1"),
            e("d", "shop.net", "kT7#vQ9!mZ2@pL5x"),
            e("e", "example.com", "kT7#vQ9!mZ2@pL5x"),
        ]);
        assert_eq!(r.reused.len(), 2);
        assert!(r.reused.contains(&vec!["a".to_owned(), "b".to_owned()]));
        assert!(r.weak.contains("c"));
        assert!(!r.weak.contains("d"));
    }

    #[test]
    fn range_answers_match_locally() {
        let (prefix, suffix) = hash("password");
        assert_eq!(prefix, "5BAA6");
        assert_eq!(suffix, "1E4C9B93F3F0682250B6CF8331B7EE68FD8");
        let answer = format!(
            "0018A45C4D1DEF81644B54AB7F969B88D65:1\r\n{suffix}:9545824\r\nFFFF0000000000000000000000000000000:0"
        );
        assert!(in_range(&answer, &suffix));
        assert!(
            !in_range(&answer, "FFFF0000000000000000000000000000000"),
            "padding"
        );
        assert!(!in_range(&answer, "0000000000000000000000000000000000A"));
    }
}
