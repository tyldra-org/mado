//! WebKit content rule lists (spec 10 §4): the filters converted with
//! adblock-rust's `into_content_blocking`, sharded under WebKit's limit of
//! 150,000 rules per list (M0 experiment 6).

use adblock::content_blocking::{CbRule, CbType};

/// WebKit's per-list limit.
pub const MAX_RULES: usize = 150_000;

/// Rules from list texts, in adblock-rust's order, and how many filters
/// had no WebKit equivalent.
pub fn convert(texts: impl IntoIterator<Item = String>) -> (Vec<CbRule>, usize) {
    // Conversion needs the raw filters, which only a debug set keeps.
    let mut set = adblock::lists::FilterSet::new(true);
    let mut filters = 0;
    for text in texts {
        filters += crate::filter_count(&text);
        set.add_filter_list(text, adblock::lists::ParseOptions::default());
    }
    match set.into_content_blocking() {
        Ok((rules, converted)) => {
            let skipped = filters.saturating_sub(converted.len());
            (rules.into_iter().filter(ascii).collect(), skipped)
        }
        Err(()) => (Vec::new(), filters),
    }
}

/// WebKit refuses non-ASCII rules.
fn ascii(r: &CbRule) -> bool {
    serde_json::to_string(r).is_ok_and(|s| s.is_ascii())
}

/// Shards of at most `max` rules: blocking and hiding rules in order, then
/// every exception (`ignore-previous-rules`) in each shard, because an
/// exception only undoes rules of its own list.
pub fn shard(rules: Vec<CbRule>, max: usize) -> Vec<String> {
    let (exceptions, rest): (Vec<CbRule>, Vec<CbRule>) = rules
        .into_iter()
        .partition(|r| r.action.typ == CbType::IgnorePreviousRules);
    let room = max.saturating_sub(exceptions.len()).max(1);
    let exceptions: Vec<String> = exceptions
        .iter()
        .filter_map(|r| serde_json::to_string(r).ok())
        .collect();
    rest.chunks(room)
        .map(|chunk| {
            let mut parts: Vec<String> = chunk
                .iter()
                .filter_map(|r| serde_json::to_string(r).ok())
                .collect();
            parts.extend(exceptions.iter().cloned());
            format!("[{}]", parts.join(","))
        })
        .collect()
}

/// A profile's own list (10 §4 step 4): its custom filters, converted, and
/// its hidden elements, `(host, selector)`, as `css-display-none` on the
/// host and its subdomains. `None` when there is nothing.
pub fn profile_rules(custom: &[String], hidden: &[(String, String)]) -> Option<String> {
    let (rules, _) = if custom.is_empty() {
        (Vec::new(), 0)
    } else {
        convert([custom.join("\n")])
    };
    let mut parts: Vec<String> = rules
        .iter()
        .filter_map(|r| serde_json::to_string(r).ok())
        .collect();
    for (host, selector) in hidden {
        if !host.is_ascii() || !selector.is_ascii() {
            continue;
        }
        parts.push(
            serde_json::json!({
                "trigger": { "url-filter": ".*", "if-domain": [format!("*{host}")] },
                "action": { "type": "css-display-none", "selector": selector },
            })
            .to_string(),
        );
    }
    (!parts.is_empty()).then(|| format!("[{}]", parts.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shards_stay_under_the_limit_and_each_carries_the_exceptions() {
        let text: String = (0..25)
            .map(|i| format!("||ads{i}.example^\n"))
            .chain([
                "@@||ads3.example/ok^\n".to_owned(),
                "example.com##.banner\n".to_owned(),
            ])
            .collect();
        let (rules, _) = convert([text]);
        let shards = shard(rules, 10);
        assert!(shards.len() >= 3);
        for s in &shards {
            let v: Vec<serde_json::Value> = serde_json::from_str(s).expect("json");
            assert!(v.len() <= 10);
            assert_eq!(
                v.last().expect("rule")["action"]["type"],
                "ignore-previous-rules"
            );
        }
        let all = shards.join("");
        assert!(all.contains("css-display-none"));
    }

    #[test]
    fn a_profile_list_holds_custom_filters_and_hidden_elements() {
        assert_eq!(profile_rules(&[], &[]), None);
        let json = profile_rules(
            &["||tracker.example^".into()],
            &[("news.example".into(), "#promo".into())],
        )
        .expect("rules");
        let v: Vec<serde_json::Value> = serde_json::from_str(&json).expect("json");
        assert!(v.len() >= 2);
        assert_eq!(v[v.len() - 1]["trigger"]["if-domain"][0], "*news.example");
        assert_eq!(v[0]["action"]["type"], "block");
    }
}
