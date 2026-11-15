//! HTTPS-first, lookalike domains, certificate and download safety decisions.
//!
//! Specification: `mado-docs/spec/08-safety-privacy.md`. Roadmap: M2-T08.

use std::collections::HashMap;
use std::io::Read;
use std::sync::LazyLock;

use mado_navigation::TopDomains;

/// Chromium's HSTS preload list, bundled gzipped until the signed lists
/// carry it (spec 08 §3, M3-T01): host, and whether subdomains are included.
static HSTS: LazyLock<HashMap<String, bool>> = LazyLock::new(|| {
    let mut text = String::new();
    let gz = include_bytes!("../data/hsts-preload.txt.gz");
    if flate2::read::GzDecoder::new(&gz[..])
        .read_to_string(&mut text)
        .is_err()
    {
        return HashMap::new();
    }
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.split(' ');
            Some((parts.next()?.to_owned(), parts.next() == Some("+")))
        })
        .collect()
});

/// Whether `host` is on the HSTS preload list, itself or under an entry
/// that includes subdomains.
pub fn hsts_preloaded(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if HSTS.contains_key(&host) {
        return true;
    }
    let mut rest = host.as_str();
    while let Some((_, parent)) = rest.split_once('.') {
        if HSTS.get(parent) == Some(&true) {
            return true;
        }
        rest = parent;
    }
    false
}

/// The sites phishing imitates most. Edit-distance and embedded-name
/// matches (08 §2 rule 3) are checked against these and the user's own
/// frequent sites only: against all 8,000 top domains they flag ordinary
/// sites (`news-today.com` is not an attack on `news.com`).
const TARGETS: &[&str] = &[
    "paypal.com",
    "apple.com",
    "icloud.com",
    "google.com",
    "gmail.com",
    "youtube.com",
    "microsoft.com",
    "live.com",
    "outlook.com",
    "office.com",
    "office365.com",
    "amazon.com",
    "facebook.com",
    "instagram.com",
    "whatsapp.com",
    "netflix.com",
    "linkedin.com",
    "twitter.com",
    "x.com",
    "dropbox.com",
    "github.com",
    "chase.com",
    "bankofamerica.com",
    "wellsfargo.com",
    "citibank.com",
    "americanexpress.com",
    "coinbase.com",
    "binance.com",
    "steamcommunity.com",
    "steampowered.com",
    "ebay.com",
    "walmart.com",
    "adobe.com",
    "docusign.com",
    "dhl.com",
    "fedex.com",
    "ups.com",
    "usps.com",
    "irs.gov",
    "hmrc.gov.uk",
    "booking.com",
    "airbnb.com",
    "spotify.com",
    "yahoo.com",
    "zoom.us",
    "slack.com",
    "salesforce.com",
    "okta.com",
];

/// What the user's history and bookmarks say about the host.
#[derive(Debug, Clone, Default)]
pub struct Known<'a> {
    /// Registrable domains with 5 or more visits.
    pub frequent: &'a [String],
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The registrable domain of an ASCII host, or the host.
fn registrable(host: &str) -> String {
    psl::domain_str(host).map_or_else(|| host.to_owned(), str::to_owned)
}

/// 08 §2: the well-known site `host` (ASCII, as in the URL) imitates, if
/// any. The caller skips hosts the user visited, bookmarked, typed in full
/// or chose to continue to.
pub fn lookalike(host: &str, known: &Known<'_>, top: &TopDomains) -> Option<String> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let domain = registrable(&host);
    let (unicode, _) = idna::domain_to_unicode(&domain);
    // Rule 1 and 2: equal confusable skeleton, with the bundled list and
    // the user's frequent sites.
    if let Some(top) = top.similar(&unicode) {
        return Some(top.to_owned());
    }
    let mine = TopDomains::from_list(&known.frequent.join("\n"));
    if let Some(site) = mine.similar(&unicode) {
        return Some(site.to_owned());
    }
    // Rule 3: one edit from a target of 6+ characters, or the target's name
    // as a word of the host's first label.
    let name = |d: &str| d.split('.').next().unwrap_or_default().to_owned();
    let label = name(&domain);
    let words: Vec<&str> = label.split('-').collect();
    TARGETS
        .iter()
        .map(|t| (*t).to_owned())
        .chain(known.frequent.iter().cloned())
        .filter(|t| *t != domain && !host.ends_with(&format!(".{t}")))
        .find(|t| {
            let target = name(t);
            (t.len() >= 6 && levenshtein(&domain, t) == 1)
                || (target.len() >= 4 && words.len() > 1 && words.contains(&target.as_str()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(host: &str) -> Option<String> {
        lookalike(host, &Known::default(), TopDomains::bundled())
    }

    #[test]
    fn lookalikes_of_well_known_sites_are_caught() {
        // pаypal.com with a Cyrillic а.
        assert_eq!(check("xn--pypal-4ve.com").as_deref(), Some("paypal.com"));
        assert_eq!(check("paypa1.com").as_deref(), Some("paypal.com"));
        assert_eq!(check("paypal-login.com").as_deref(), Some("paypal.com"));
        assert_eq!(
            check("secure.paypal-login.com").as_deref(),
            Some("paypal.com")
        );
        assert_eq!(check("gooogle.com").as_deref(), Some("google.com"));
    }

    #[test]
    fn real_and_ordinary_sites_are_left_alone() {
        for host in [
            "paypal.com",
            "www.paypal.com",
            "news-today.com",
            "example.org",
            "rust-lang.org",
            "apple.co.uk",
        ] {
            assert_eq!(check(host), None, "{host}");
        }
    }

    #[test]
    fn the_hsts_preload_list_covers_subdomains_where_it_says_so() {
        assert!(hsts_preloaded("accounts.google.com"));
        assert!(
            hsts_preloaded("sso.accounts.google.com"),
            "included subdomains"
        );
        assert!(hsts_preloaded("paypal.com"));
        assert!(
            !hsts_preloaded("shop.paypal.com"),
            "paypal.com does not include subdomains"
        );
        assert!(!hsts_preloaded("example.com"));
        assert!(!hsts_preloaded("expired.mado.test"));
    }

    #[test]
    fn the_users_own_frequent_sites_count() {
        let frequent = vec!["mycorp-intranet.com".to_owned()];
        let known = Known {
            frequent: &frequent,
        };
        assert_eq!(
            lookalike("mycorp-intranct.com", &known, TopDomains::bundled()).as_deref(),
            Some("mycorp-intranet.com")
        );
    }
}
