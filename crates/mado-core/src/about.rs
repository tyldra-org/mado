//! About (spec 15 §5): whether the web engine under Mado is older than the
//! compatibility list's `minSafeEngine`, from the bundled copy until the
//! signed list arrives (M3-T01).

/// `"620.1.15"` as numbers; anything not a number counts as 0.
fn parts(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// WebKit's version without macOS's prefix: `22625.2.8.11.9` is
/// `625.2.8.11.9`.
pub fn engine_version(bundle_version: &str) -> String {
    let mut it = bundle_version.splitn(2, '.');
    let first = it.next().unwrap_or_default();
    let rest = it.next();
    let major = match first.parse::<u64>() {
        Ok(n) if n >= 10_000 => (n % 1000).to_string(),
        _ => first.to_owned(),
    };
    match rest {
        Some(r) => format!("{major}.{r}"),
        None => major,
    }
}

/// Whether `engine` is older than `min`.
pub fn older(engine: &str, min: &str) -> bool {
    let (a, b) = (parts(engine), parts(min));
    let n = a.len().max(b.len());
    let get = |v: &[u64], i: usize| v.get(i).copied().unwrap_or(0);
    (0..n)
        .map(|i| get(&a, i).cmp(&get(&b, i)))
        .find(|o| o.is_ne())
        .is_some_and(std::cmp::Ordering::is_lt)
}

/// Whether `engine` (an engine version) is older than the bundled list's
/// `minSafeEngine.webkit`. An unknown version is not reported.
pub fn engine_outdated(engine: &str) -> bool {
    let list: serde_json::Value =
        serde_json::from_str(include_str!("../data/compat.json")).unwrap_or_default();
    match list["minSafeEngine"]["webkit"].as_str() {
        Some(min) if !engine.is_empty() => older(engine, min),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_without_the_os_prefix() {
        assert_eq!(engine_version("22625.2.8.11.9"), "625.2.8.11.9");
        assert_eq!(engine_version("625.1"), "625.1");
        assert!(older("619.9", "620.1.15"));
        assert!(older("620.1.14.9", "620.1.15"));
        assert!(!older("620.1.15", "620.1.15"));
        assert!(!older("625.2.8.11.9", "620.1.15"));
        assert!(!engine_outdated(""));
        assert!(engine_outdated("600.1"));
        assert!(!engine_outdated("625.2.8"));
    }
}
