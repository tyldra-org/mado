//! IDN display (spec 05 §4): whether a punycode label may show as Unicode.
//! A port of Chromium's `IDNSpoofChecker` and `SkeletonGenerator`
//! (components/url_formatter, BSD-3-Clause) without ICU: Unicode TR39
//! confusables and identifier status are bundled in `data/`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};
use unicode_script::{Script, UnicodeScript};

/// TR39 confusables: each character's prototype (its skeleton piece).
static CONFUSABLES: LazyLock<HashMap<char, String>> = LazyLock::new(|| {
    let hex = |s: &str| u32::from_str_radix(s, 16).ok().and_then(char::from_u32);
    include_str!("../data/confusables.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (from, to) = l.split_once(';')?;
            let to: Option<String> = to.split_whitespace().map(hex).collect();
            Some((hex(from.trim())?, to?))
        })
        .collect()
});

/// TR39 IdentifierStatus=Allowed, minus the characters Chromium removes.
static ALLOWED: LazyLock<Vec<(u32, u32)>> = LazyLock::new(|| {
    let mut ranges: Vec<(u32, u32)> = include_str!("../data/identifier-allowed.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (a, b) = l.split_once("..").unwrap_or((l, l));
            Some((
                u32::from_str_radix(a, 16).ok()?,
                u32::from_str_radix(b, 16).ok()?,
            ))
        })
        .collect();
    ranges.sort_unstable();
    ranges
});

const REMOVED: &[(u32, u32)] = &[
    (0x338, 0x338),
    (0x58a, 0x58a),
    (0x2010, 0x2010),
    (0x2019, 0x2019),
    (0x2027, 0x2027),
    (0x30a0, 0x30a0),
    (0x2bb, 0x2bc),
    (0x2ec, 0x2ec),
    (0x138, 0x138),
    // Blank in the macOS system font.
    (0x620, 0x620),
    (0xf8c, 0xf8f),
    // Rarely used LGC blocks.
    (0x1cd, 0x1dc),
    (0x1c80, 0x1c8f),
    (0x1e00, 0x1e9b),
    (0x1f00, 0x1fff),
    (0xa640, 0xa69f),
    (0xa720, 0xa7ff),
];

fn in_ranges(ranges: &[(u32, u32)], c: u32) -> bool {
    let i = ranges.partition_point(|r| r.1 < c);
    ranges.get(i).is_some_and(|r| r.0 <= c)
}

fn allowed(c: char) -> bool {
    let c = c as u32;
    in_ranges(&ALLOWED, c) && !REMOVED.iter().any(|r| r.0 <= c && c <= r.1)
}

/// Scripts of a character for restriction levels: its Script_Extensions,
/// with Han, Kana and Hangul augmented as TR39 §5.1 does, or `None` for
/// Common and Inherited.
fn scripts(c: char) -> Option<BTreeSet<&'static str>> {
    let scx = c.script_extension();
    if scx.is_common() || scx.is_inherited() {
        return None;
    }
    let mut out = BTreeSet::new();
    for s in scx.iter() {
        out.insert(s.short_name());
        match s {
            Script::Han => {
                out.extend(["Hanb", "Jpan", "Kore"]);
            }
            Script::Hiragana | Script::Katakana => {
                out.insert("Jpan");
            }
            Script::Hangul => {
                out.insert("Kore");
            }
            Script::Bopomofo => {
                out.insert("Hanb");
            }
            _ => {}
        }
    }
    Some(out)
}

fn resolved(label: &str, without_latin: bool) -> Option<BTreeSet<&'static str>> {
    let mut acc: Option<BTreeSet<&'static str>> = None;
    for c in label.chars() {
        let Some(s) = scripts(c) else { continue };
        if without_latin && s.contains("Latn") {
            continue;
        }
        acc = Some(match acc {
            None => s,
            Some(a) => a.intersection(&s).copied().collect(),
        });
    }
    acc
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Ascii,
    SingleScript,
    HighlyRestrictive,
    Weaker,
}

fn restriction_level(label: &str) -> Level {
    if label.is_ascii() {
        return Level::Ascii;
    }
    if resolved(label, false).is_none_or(|s| !s.is_empty()) {
        return Level::SingleScript;
    }
    let rest = resolved(label, true).unwrap_or_default();
    if ["Kore", "Hanb", "Jpan"].iter().any(|s| rest.contains(s)) {
        return Level::HighlyRestrictive;
    }
    Level::Weaker
}

/// ICU's `uspoof_check` as Chromium configures it: allowed characters,
/// highly restrictive script mixing, no repeated nonspacing marks, one
/// numbering system.
fn icu_checks_pass(label: &str) -> bool {
    if !label.chars().all(allowed) {
        return false;
    }
    if restriction_level(label) > Level::HighlyRestrictive {
        return false;
    }
    // Invisible: the same nonspacing mark twice on one base.
    let mut marks: Vec<char> = Vec::new();
    for c in label.nfd() {
        if c.general_category() == GeneralCategory::NonspacingMark {
            if marks.contains(&c) {
                return false;
            }
            marks.push(c);
        } else {
            marks.clear();
        }
    }
    // Mixed numbers: digits from more than one numbering system.
    let zeros: HashSet<u32> = label
        .chars()
        .filter(|c| c.general_category() == GeneralCategory::DecimalNumber)
        .map(|c| {
            let mut start = c as u32;
            while start > 0
                && char::from_u32(start - 1)
                    .is_some_and(|p| p.general_category() == GeneralCategory::DecimalNumber)
            {
                start -= 1;
            }
            c as u32 - (c as u32 - start) % 10
        })
        .collect();
    zeros.len() <= 1
}

struct WholeScript {
    script: Script,
    lookalikes: &'static str,
    tlds: &'static [&'static str],
}

const WHOLE_SCRIPT: &[WholeScript] = &[
    WholeScript {
        script: Script::Armenian,
        lookalikes: "ագզէլհյոսւօՙ",
        tlds: &["am"],
    },
    WholeScript {
        script: Script::Cyrillic,
        lookalikes: "аысԁеԍһіюкјӏморԗԛѕтԝхуъьҽпгѵѡ",
        tlds: &["bg", "by", "kz", "pyc", "ru", "su", "ua", "uz"],
    },
    WholeScript {
        script: Script::Ethiopic,
        lookalikes: "ሀሠሰስበነተከዐዕዘጠፐꬅ",
        tlds: &["er", "et"],
    },
    WholeScript {
        script: Script::Georgian,
        lookalikes: "იოყძხჽჿ",
        tlds: &["ge"],
    },
    WholeScript {
        script: Script::Greek,
        lookalikes: "αικνρυωηοτ",
        tlds: &["gr"],
    },
    WholeScript {
        script: Script::Hebrew,
        lookalikes: "דוחיןסװײ׳ﬦ",
        tlds: &["il"],
    },
    WholeScript {
        script: Script::Bengali,
        lookalikes: "০৭",
        tlds: &[],
    },
    WholeScript {
        script: Script::Devanagari,
        lookalikes: "ऽ०ॱ",
        tlds: &[],
    },
    WholeScript {
        script: Script::Gujarati,
        lookalikes: "ડટ૦૧",
        tlds: &[],
    },
    WholeScript {
        script: Script::Gurmukhi,
        lookalikes: "੦੧",
        tlds: &[],
    },
    WholeScript {
        script: Script::Kannada,
        lookalikes: "ಽ೦೧",
        tlds: &[],
    },
    WholeScript {
        script: Script::Malayalam,
        lookalikes: "ടഠധനറ൦",
        tlds: &[],
    },
    WholeScript {
        script: Script::Oriya,
        lookalikes: "ଠ୦୮",
        tlds: &[],
    },
    WholeScript {
        script: Script::Tamil,
        lookalikes: "டப௦",
        tlds: &[],
    },
    WholeScript {
        script: Script::Telugu,
        lookalikes: "౦౧",
        tlds: &[],
    },
    WholeScript {
        script: Script::Myanmar,
        lookalikes: "ခဂငထပဝ၀၂ၔၜ\u{1090}\u{1091}\u{1095}\u{1096}\u{1097}",
        tlds: &["mm"],
    },
    WholeScript {
        script: Script::Thai,
        lookalikes: "บพเแ๐",
        tlds: &["th"],
    },
];

const ALLOWED_WHOLE_SCRIPT_WORDS: &[&str] = &[
    "секс",
    "как",
    "коса",
    "курс",
    "парк",
    "такий",
    "укроп",
    "сахарок",
    "покраска",
    "театр",
    "астро",
    "пхукет",
];

const DIGIT_LOOKALIKES: &str = "θ२২੨੨૨೩೭շзҙӡउওਤ੩૩౩ဒვპੜკ੫丩ㄐճ৪੪୫૭୨౨";

fn digit_lookalike(label: &str) -> bool {
    let mut lookalike = false;
    for c in label.chars() {
        if c.is_ascii_digit() {
            continue;
        }
        if DIGIT_LOOKALIKES.contains(c) {
            lookalike = true;
            continue;
        }
        return false;
    }
    lookalike
}

static DANGEROUS: LazyLock<Regex> = LazyLock::new(|| {
    let kana = r"\p{scx=Katakana}";
    let hira = r"\p{scx=Hiragana}";
    let hani = r"\p{scx=Han}";
    let bopo = r"\p{scx=Bopomofo}";
    let cjk_lookalikes = r"[\x{4e00}\x{3127}\x{4e28}\x{4e5b}\x{4e03}\x{4e05}\x{5341}\x{3007}\x{3112}\x{311a}\x{311f}\x{3128}\x{3129}\x{3108}\x{31ba}\x{31b3}\x{5de5}\x{31b2}\x{8ba0}\x{4e01}\x{4e36}\x{4e85}\x{4e8c}\x{4ea0}\x{5196}\x{5b80}\x{5ddb}]";
    let parts = [
        format!(
            r"[^{kana}{hira}{hani}][\x{{30ce}}\x{{30f3}}\x{{30bd}}\x{{30be}}\x{{4e36}}\x{{4e40}}\x{{4e41}}\x{{4e3f}}][^{kana}{hira}{hani}]"
        ),
        format!(r"[^{kana}][\x{{30fd}}\x{{30fe}}]"),
        r"^[\x{30fd}\x{30fe}]".to_owned(),
        format!(r"^[{kana}]+[\x{{3078}}-\x{{307a}}][{kana}]+$"),
        format!(r"^[{hira}]+[\x{{30d8}}-\x{{30da}}][{hira}]+$"),
        format!(r"[^{kana}{hira}]\x{{30fc}}"),
        r"^\x{30fc}".to_owned(),
        format!(r"[^{kana}{hira}{hani}]\x{{30fb}}"),
        format!(r"\x{{30fb}}[^{kana}{hira}{hani}]"),
        r"\x{30fb}$".to_owned(),
        format!(r"[^{kana}{hira}{hani}{bopo}]{cjk_lookalikes}"),
        format!(r"{cjk_lookalikes}[^{kana}{hira}{hani}{bopo}]"),
        r"[^\p{scx=Latin}\p{scx=Greek}\p{scx=Cyrillic}][\x{0300}-\x{0339}]".to_owned(),
        r"\x{0131}[\x{0300}-\x{0339}]".to_owned(),
        r"\x{3099}|\x{309a}".to_owned(),
        r"[ijl]\x{0307}".to_owned(),
    ];
    Regex::new(&parts.join("|")).expect("the dangerous pattern compiles")
});

fn unsafe_middle_dot(label: &[char], tld: &str) -> bool {
    for (i, &c) in label.iter().enumerate() {
        if c != '·' {
            continue;
        }
        if tld != "cat" || i == 0 || i == label.len() - 1 {
            return true;
        }
        if label[i - 1] != 'l' || label[i + 1] != 'l' {
            return true;
        }
    }
    false
}

/// [[:Latin:][:Greek:][:Cyrillic:][0-9._-][\u0300-\u0339]]
fn lgc_or_ascii(c: char) -> bool {
    matches!(c.script(), Script::Latin | Script::Greek | Script::Cyrillic)
        || c.is_ascii_digit()
        || matches!(c, '.' | '_' | '-')
        || ('\u{300}'..='\u{339}').contains(&c)
}

/// Chromium's `SafeToDisplayAsUnicode` for one decoded label.
fn safe_label(label: &str, tld: &str, tld_unicode: &str) -> bool {
    if !icu_checks_pass(label) {
        return false;
    }
    let chars: Vec<char> = label.chars().collect();
    if chars.len() > 1 && tld != "is" && tld != "fo" && label.contains(['\u{fe}', '\u{f0}']) {
        return false;
    }
    if chars.len() > 1 && tld != "az" && label.contains('ə') {
        return false;
    }
    if unsafe_middle_dot(&chars, tld) {
        return false;
    }
    let level = restriction_level(label);
    if level == Level::Ascii {
        return true;
    }
    if level == Level::SingleScript
        && !label.chars().any(|c| matches!(c, '\u{3078}'..='\u{307a}' | '\u{30d8}'..='\u{30da}' | '\u{30fb}'..='\u{30fe}'))
        && !label.chars().any(|c| ('\u{300}'..='\u{339}').contains(&c))
    {
        for ws in WHOLE_SCRIPT {
            let of_script: Vec<char> = label.chars().filter(|c| c.script() == ws.script).collect();
            let confusable = !of_script.is_empty() && of_script.iter().all(|c| ws.lookalikes.contains(*c));
            let tld_allowed =
                tld_unicode.chars().any(|c| c.script() == ws.script) || ws.tlds.contains(&tld);
            if confusable && !tld_allowed && !ALLOWED_WHOLE_SCRIPT_WORDS.contains(&label) {
                return false;
            }
        }
        return !digit_lookalike(label);
    }
    if digit_lookalike(label) {
        return false;
    }
    let non_ascii_latin = label
        .chars()
        .any(|c| c.script() == Script::Latin && !c.is_ascii_alphabetic());
    if non_ascii_latin && !label.chars().all(lgc_or_ascii) {
        return false;
    }
    !DANGEROUS.is_match(label)
}

// Skeletons, for lookalikes of top domains.

fn remove_diacritics(host: &str) -> String {
    let host = host.strip_suffix('.').unwrap_or(host);
    if !host.chars().all(lgc_or_ascii) {
        return host.to_owned();
    }
    let stripped: String = host
        .nfd()
        .filter(|c| c.general_category() != GeneralCategory::NonspacingMark)
        .nfc()
        .collect();
    stripped
        .replace('ł', "l")
        .replace('ø', "o")
        .replace('đ', "d")
}

fn multiple_skeletons(c: char) -> &'static [&'static str] {
    match c {
        'þ' => &["b", "p"],
        'œ' => &["ce", "oe"],
        'ł' => &["l", "t"],
        'ı' => &["l", "i"],
        _ => &[],
    }
}

/// Every spelling of `input` with each character of several skeletons
/// replaced or kept (Chromium's supplemental hostnames).
fn supplemental(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    if chars.is_empty()
        || chars.len() > 32
        || chars
            .iter()
            .filter(|c| !multiple_skeletons(**c).is_empty())
            .count()
            > 5
    {
        return Vec::new();
    }
    let mut out = vec![String::new()];
    for c in chars {
        let mut next = Vec::new();
        for prefix in &out {
            next.push(format!("{prefix}{c}"));
            for alt in multiple_skeletons(c) {
                next.push(format!("{prefix}{alt}"));
            }
        }
        out = next;
        out.truncate(128);
    }
    out
}

fn extra_confusable(c: char) -> Option<&'static str> {
    const MAP: &[(&str, &str)] = &[
        ("æӕ", "ae"),
        ("ϼҏ", "p"),
        ("ħнћңҥӈӊԋԧԩ", "h"),
        ("ĸκкқҝҟҡӄԟ", "k"),
        ("ŋпԥกח", "n"),
        ("ŧтҭԏ七丅丆丁", "t"),
        ("ƅьҍвß", "b"),
        ("ωшщพฟພຟ", "w"),
        ("мӎ", "m"),
        ("єҽҿၔ", "e"),
        ("ґ", "r"),
        ("ғӻ", "f"),
        ("ҫင", "c"),
        ("ұ丫", "y"),
        ("χҳӽӿ乂", "x"),
        ("ԃძ", "d"),
        ("ԍဌ", "g"),
        ("ടรຣຮ", "s"),
        ("၂", "j"),
        ("०০੦૦ଠ୦೦စ", "o"),
        ("৭੧૧", "q"),
        ("บບ", "u"),
        ("θ", "0"),
        ("२২੨૨೩೭շ", "2"),
        ("зҙӡउওਤ੩૩౩ဒვპੜკ", "3"),
        ("੫丩ㄐ", "4"),
        ("ճ", "6"),
        ("৪੪୫", "8"),
        ("૭୨౨", "9"),
        ("—一―⸺⸻", "-"),
    ];
    MAP.iter()
        .find(|(set, _)| set.contains(c))
        .map(|(_, to)| *to)
}

/// ICU's skeleton: NFD, each character to its TR39 prototype, NFD again.
fn skeleton(s: &str) -> String {
    let mut out = String::new();
    for c in s.nfd() {
        match CONFUSABLES.get(&c) {
            Some(p) => out.push_str(p),
            None => out.push(c),
        }
    }
    out.nfd().collect()
}

fn skeletons(host: &str) -> BTreeSet<String> {
    let mut variants: BTreeSet<String> =
        supplemental(&remove_diacritics(host)).into_iter().collect();
    if host.contains('ł') {
        variants.extend(supplemental(host));
    }
    let mut out = BTreeSet::new();
    for v in variants {
        let v = v.strip_suffix('.').unwrap_or(&v);
        let mapped: String = v
            .chars()
            .map(|c| extra_confusable(c).map_or_else(|| c.to_string(), str::to_owned))
            .collect();
        if mapped.contains('ӏ') {
            out.insert(skeleton(&mapped.replace('ӏ', "l")));
        }
        out.insert(skeleton(&mapped));
    }
    out
}

/// Domains people know, by skeleton, for lookalike checks.
#[derive(Debug, Default)]
pub struct TopDomains {
    by_skeleton: HashMap<String, String>,
}

static BUNDLED: LazyLock<TopDomains> =
    LazyLock::new(|| TopDomains::from_list(include_str!("../data/top-domains.txt")));

impl TopDomains {
    /// Chromium's list of top domains, bundled.
    pub fn bundled() -> &'static Self {
        &BUNDLED
    }

    /// One domain per line; `#` starts a comment line.
    pub fn from_list(list: &str) -> Self {
        let mut by_skeleton = HashMap::new();
        for domain in list
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            for s in skeletons(domain) {
                by_skeleton.entry(s).or_insert_with(|| domain.to_owned());
            }
        }
        Self { by_skeleton }
    }

    fn lookup(&self, skeleton: &str) -> Option<&str> {
        let labels: Vec<&str> = skeleton.split('.').collect();
        let mut labels = &labels[labels.len().saturating_sub(4)..];
        while labels.len() >= 2 {
            if let Some(d) = self.by_skeleton.get(&labels.join(".")) {
                return Some(d);
            }
            labels = &labels[1..];
        }
        None
    }

    /// A top domain `host` (Unicode) looks like without being it or under
    /// it, by confusable skeleton.
    pub fn similar(&self, host: &str) -> Option<&str> {
        for s in skeletons(host) {
            if s.is_empty() {
                continue;
            }
            if let Some(top) = self.lookup(&s) {
                let host = host.strip_suffix('.').unwrap_or(host);
                if host.eq_ignore_ascii_case(top)
                    || host.to_lowercase().ends_with(&format!(".{top}"))
                {
                    return None;
                }
                return Some(top);
            }
        }
        None
    }
}

fn decode_label(label: &str) -> Option<String> {
    let (out, result) = idna::domain_to_unicode(label);
    // UTS 46 CheckHyphens: no "--" in the third and fourth positions.
    let hyphens =
        out.chars().skip(2).take(2).eq("--".chars()) || out.starts_with('-') || out.ends_with('-');
    (result.is_ok() && !out.contains('.') && !hyphens).then_some(out)
}

/// The host as people should read it: punycode labels decoded only when
/// they pass the spoof checks, and the whole host left in punycode when it
/// looks like a top domain. Returns the text and whether any label stays
/// in punycode.
pub fn host_for_display(host: &str, top: &TopDomains) -> (String, bool) {
    let host = host.to_ascii_lowercase();
    let tld = host
        .rsplit_once('.')
        .map(|(_, t)| t)
        .filter(|t| !t.is_empty())
        .unwrap_or("");
    let tld_unicode = if tld.starts_with("xn--") {
        decode_label(tld).unwrap_or_else(|| tld.to_owned())
    } else {
        tld.to_owned()
    };
    let mut out = Vec::new();
    let mut had_idn = false;
    let mut kept = false;
    for label in host.split('.') {
        if !label.starts_with("xn--") || label.ends_with('-') {
            out.push(label.to_owned());
            continue;
        }
        match decode_label(label) {
            Some(u) => {
                had_idn = true;
                if safe_label(&u, tld, &tld_unicode) {
                    out.push(u);
                } else {
                    kept = true;
                    out.push(label.to_owned());
                }
            }
            None => {
                kept = true;
                out.push(label.to_owned());
            }
        }
    }
    let text = out.join(".");
    if had_idn && top.similar(&text).is_some() {
        return (host, true);
    }
    (text, kept)
}
