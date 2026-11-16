//! The password generator (spec 11 §5): 20 characters from `A-Z a-z 0-9`
//! and `-_.!@#`, at least one of each class, honouring the page's
//! `passwordrules` attribute (the WHATWG proposal Safari implements).

const UPPER: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ";
const LOWER: &str = "abcdefghijkmnopqrstuvwxyz";
const DIGIT: &str = "23456789";
const SPECIAL: &str = "-_.!@#";

/// What a page's `passwordrules` asks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rules {
    /// Classes that must each appear at least once.
    pub required: Vec<String>,
    /// Characters that may appear (empty: the default set).
    pub allowed: String,
    pub min: Option<usize>,
    pub max: Option<usize>,
    pub max_consecutive: Option<usize>,
}

fn class(name: &str) -> Option<String> {
    let name = name.trim();
    Some(match name {
        "upper" => UPPER.into(),
        "lower" => LOWER.into(),
        "digit" => DIGIT.into(),
        "special" => SPECIAL.into(),
        "ascii-printable" | "unicode" => format!("{UPPER}{LOWER}{DIGIT}{SPECIAL}"),
        custom if custom.starts_with('[') && custom.ends_with(']') => {
            let inner = &custom[1..custom.len() - 1];
            let set: String = inner.chars().filter(|c| c.is_ascii_graphic()).collect();
            if set.is_empty() {
                return None;
            }
            set
        }
        _ => return None,
    })
}

/// Reads a `passwordrules` value; unknown parts are ignored.
pub fn parse_rules(text: &str) -> Rules {
    let mut r = Rules::default();
    for part in text.split(';') {
        let Some((key, value)) = part.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "required" => {
                let set: String = value.split(',').filter_map(class).collect();
                if !set.is_empty() {
                    r.required.push(set);
                }
            }
            "allowed" => r.allowed.extend(value.split(',').filter_map(class)),
            "minlength" => r.min = value.parse().ok(),
            "maxlength" => r.max = value.parse().ok(),
            "max-consecutive" => r.max_consecutive = value.parse().ok(),
            _ => {}
        }
    }
    r
}

fn pick(set: &[char], random: &mut impl FnMut(u32) -> u32) -> char {
    let n = u32::try_from(set.len()).unwrap_or(1).max(1);
    set[usize::try_from(random(n)).unwrap_or(0) % set.len().max(1)]
}

/// A password for `rules`. `random(n)` returns a uniform number below `n`.
pub fn generate_with(rules: &Rules, random: &mut impl FnMut(u32) -> u32) -> String {
    let mut groups: Vec<Vec<char>> = if rules.required.is_empty() {
        [UPPER, LOWER, DIGIT, SPECIAL]
            .iter()
            .map(|s| s.chars().collect())
            .collect()
    } else {
        rules.required.iter().map(|s| s.chars().collect()).collect()
    };
    let mut all: Vec<char> = if rules.allowed.is_empty() {
        groups.iter().flatten().copied().collect()
    } else {
        rules.allowed.chars().collect()
    };
    for g in &groups {
        all.extend(g);
    }
    all.sort_unstable();
    all.dedup();
    groups.retain(|g| !g.is_empty());
    let len = 20
        .max(rules.min.unwrap_or(0))
        .min(rules.max.unwrap_or(usize::MAX))
        .max(groups.len());
    let max_run = rules.max_consecutive.map(|m| m.max(1));
    for _ in 0..100 {
        let mut chars: Vec<char> = groups.iter().map(|g| pick(g, random)).collect();
        while chars.len() < len {
            chars.push(pick(&all, random));
        }
        // Fisher-Yates, so the required characters are not always first.
        for i in (1..chars.len()).rev() {
            let j = usize::try_from(random(u32::try_from(i + 1).unwrap_or(1))).unwrap_or(0);
            chars.swap(i, j);
        }
        let run_ok =
            max_run.is_none_or(|m| chars.windows(m + 1).all(|w| w.iter().any(|c| *c != w[0])));
        if run_ok {
            return chars.into_iter().collect();
        }
    }
    String::new()
}

/// A password from the OS random number generator.
pub fn generate(rules: &Rules) -> String {
    generate_with(rules, &mut |n| {
        let mut b = [0u8; 4];
        // The OS generator does not fail on supported systems.
        getrandom::fill(&mut b).expect("the OS random number generator");
        // Rejection sampling keeps every value equally likely.
        let zone = u32::MAX - u32::MAX % n.max(1);
        let mut v = u32::from_le_bytes(b);
        while v >= zone {
            getrandom::fill(&mut b).expect("the OS random number generator");
            v = u32::from_le_bytes(b);
        }
        v % n.max(1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_has_every_class_and_twenty_characters() {
        for _ in 0..50 {
            let p = generate(&Rules::default());
            assert_eq!(p.chars().count(), 20);
            for set in [UPPER, LOWER, DIGIT, SPECIAL] {
                assert!(p.chars().any(|c| set.contains(c)), "{p} lacks one of {set}");
            }
        }
    }

    #[test]
    fn passwordrules_are_honoured() {
        let r = parse_rules(
            "required: lower; required: digit; allowed: [ab12]; maxlength: 12; max-consecutive: 2;",
        );
        assert_eq!(r.max, Some(12));
        for _ in 0..50 {
            let p = generate(&r);
            assert_eq!(p.chars().count(), 12);
            assert!(
                p.chars()
                    .all(|c| "ab12".contains(c) || LOWER.contains(c) || DIGIT.contains(c))
            );
            assert!(p.chars().any(|c| DIGIT.contains(c)));
            assert!(!p.chars().any(|c| SPECIAL.contains(c) || UPPER.contains(c)));
        }
        let short = generate(&parse_rules("minlength: 30; required: upper, digit"));
        assert_eq!(short.chars().count(), 30);
        assert!(
            short
                .chars()
                .any(|c| UPPER.contains(c) || DIGIT.contains(c))
        );
    }
}
