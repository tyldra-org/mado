//! HTTPS-first, lookalike domains, certificate and download safety decisions.
//!
//! Specification: `mado-docs/spec/08-safety-privacy.md`. Roadmap: M2-T08.

use mado_navigation::TopDomains;

/// Whether `host` is on Chromium's HSTS preload list (spec 08 §3), itself
/// or under an entry that includes subdomains.
pub fn hsts_preloaded(host: &str) -> bool {
    mado_lists::hsts_preload().preloaded(host)
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

/// A link without its tracking parameters (`utm_*`, `fbclid`, `gclid`,
/// `si` on youtu.be and the rest of `lists/tracking-params.json`), for
/// Copy clean link (spec 08 §8).
pub fn clean_link(url: &str) -> String {
    let Ok(mut u) = url::Url::parse(url) else {
        return url.to_owned();
    };
    let list = mado_lists::tracking_params();
    let host = u.host_str().unwrap_or_default().to_ascii_lowercase();
    let on_host: Vec<&str> = list
        .hosts
        .iter()
        .filter(|(h, _)| host == **h || host.ends_with(&format!(".{h}")))
        .flat_map(|(_, p)| p.iter().map(String::as_str))
        .collect();
    let tracking = |k: &str| {
        list.prefixes.iter().any(|p| k.starts_with(p.as_str()))
            || list.params.iter().any(|p| p == k)
            || on_host.contains(&k)
    };
    if u.query().is_none() {
        return u.to_string();
    }
    let kept: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| !tracking(k))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        u.set_query(None);
    } else {
        u.query_pairs_mut().clear().extend_pairs(kept);
    }
    u.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(host: &str) -> Option<String> {
        lookalike(host, &Known::default(), &TopDomains::current())
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
            lookalike("mycorp-intranct.com", &known, &TopDomains::current()).as_deref(),
            Some("mycorp-intranet.com")
        );
    }

    #[test]
    fn clean_links_lose_tracking() {
        assert_eq!(
            clean_link("https://a.example/p?utm_source=x&id=7&fbclid=y"),
            "https://a.example/p?id=7"
        );
        assert_eq!(
            clean_link("https://a.example/p?gclid=1"),
            "https://a.example/p"
        );
        assert_eq!(
            clean_link("https://youtu.be/dQw4w9WgXcQ?si=abc&t=42"),
            "https://youtu.be/dQw4w9WgXcQ?t=42"
        );
        assert_eq!(
            clean_link("https://open.spotify.com/track/1?si=x"),
            "https://open.spotify.com/track/1"
        );
        // `si` means something else elsewhere.
        assert_eq!(
            clean_link("https://a.example/?si=1"),
            "https://a.example/?si=1"
        );
        assert_eq!(clean_link("not a url"), "not a url");
    }
}
