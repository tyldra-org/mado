//! Memory (spec 13 §10): Markdown files in `<profile>/AI/memory/`, a
//! filter that keeps secrets out, paragraphs for the search index, and the
//! daily filing of `inbox.md` by the `fast` model.

use std::collections::BTreeMap;

use mado_ai::{Effort, Message, ModelRequest};

/// The inbox `remember` appends to.
pub const INBOX: &str = "inbox.md";

/// A memory file's path, relative to the memory folder: `USER.md`,
/// `MEMORY.md`, `inbox.md`, or a Markdown file in `people/`, `sites/` or
/// `projects/`.
pub fn valid_path(path: &str) -> bool {
    match path.split_once('/') {
        None => matches!(path, "USER.md" | "MEMORY.md" | INBOX),
        Some((dir, file)) => {
            matches!(dir, "people" | "sites" | "projects")
                && file.ends_with(".md")
                && file.len() > 3
                && !file.starts_with('.')
                && file
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        }
    }
}

/// Why text was not remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    CardNumber,
    ApiKey,
    Password,
    IdNumber,
}

impl Refused {
    pub fn message_key(self) -> &'static str {
        match self {
            Self::CardNumber => "error.ai.memoryCard",
            Self::ApiKey => "error.ai.memoryKey",
            Self::Password => "error.ai.memoryPassword",
            Self::IdNumber => "error.ai.memoryId",
        }
    }
}

fn luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                if d * 2 > 9 { d * 2 - 9 } else { d * 2 }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// Runs of 13 to 19 digits, spaces and dashes allowed between them, that
/// pass the Luhn check.
fn has_card(text: &str) -> bool {
    let mut run: Vec<u32> = Vec::new();
    let check = |run: &[u32]| (13..=19).contains(&run.len()) && luhn(run);
    let mut gap = 0;
    for c in text.chars() {
        if let Some(d) = c.to_digit(10) {
            run.push(d);
            gap = 0;
        } else if matches!(c, ' ' | '-') && !run.is_empty() && gap == 0 {
            gap += 1;
        } else {
            if check(&run) {
                return true;
            }
            run.clear();
            gap = 0;
        }
    }
    check(&run)
}

fn has_key(text: &str) -> bool {
    text.split(|c: char| {
        c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '(' | ')' | '<' | '>')
    })
    .any(|w| {
        let long = w.len() >= 20;
        (w.starts_with("sk-") && w.len() >= 20)
            || (w.starts_with("AKIA")
                && w.len() == 20
                && w.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
            || ([
                "ghp_",
                "gho_",
                "ghs_",
                "github_pat_",
                "xoxb-",
                "xoxp-",
                "xoxa-",
                "xoxs-",
            ]
            .iter()
            .any(|p| w.starts_with(p))
                && long)
            || (w.len() >= 32
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-'))
                && w.chars().any(|c| c.is_ascii_digit())
                && w.chars().any(|c| c.is_ascii_uppercase())
                && w.chars().any(|c| c.is_ascii_lowercase()))
    })
}

fn has_id(text: &str) -> bool {
    let b = text.as_bytes();
    // US Social Security numbers: 123-45-6789.
    let ssn = (0..b.len().saturating_sub(10)).any(|i| {
        let w = &b[i..i + 11];
        let edge = |j: usize| b.get(j).is_none_or(|c| !c.is_ascii_digit());
        w.iter().enumerate().all(|(k, c)| {
            if k == 3 || k == 6 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        }) && (i == 0 || edge(i - 1))
            && edge(i + 11)
    });
    // IBANs: two letters, two digits, 11 to 30 letters and digits.
    let iban = text.split_whitespace().any(|w| {
        let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        (15..=34).contains(&w.len())
            && w[..2].chars().all(|c| c.is_ascii_uppercase())
            && w[2..4].chars().all(|c| c.is_ascii_digit())
            && w.chars().all(|c| c.is_ascii_alphanumeric())
    });
    ssn || iban
}

/// 13 §10: text with a card number, an API key, one of the vault's
/// passwords or an ID number is not written.
pub fn refuse(text: &str, passwords: &[String]) -> Option<Refused> {
    if has_card(text) {
        return Some(Refused::CardNumber);
    }
    if has_key(text) {
        return Some(Refused::ApiKey);
    }
    if passwords
        .iter()
        .any(|p| p.chars().count() >= 6 && text.contains(p.as_str()))
    {
        return Some(Refused::Password);
    }
    if has_id(text) {
        return Some(Refused::IdNumber);
    }
    None
}

/// `(YYYY-MM-DD, YYYY-MM-DD HH:MM)` in UTC for Unix milliseconds.
pub fn utc(ms: i64) -> (String, String) {
    let secs = ms.div_euclid(1000);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let day = format!("{y:04}-{m:02}-{d:02}");
    let time = format!("{day} {:02}:{:02}", rest / 3600, rest % 3600 / 60);
    (day, time)
}

/// An inbox line: `- <time> (task <id>) <text>`, on one line.
pub fn inbox_entry(time: &str, task: &str, text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("- {time} (task {task}) {text}\n")
}

/// One paragraph of a memory file, for the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paragraph {
    pub heading: String,
    /// The paragraph's first line, from 1.
    pub line: usize,
    pub body: String,
}

/// A file's paragraphs (blank-line separated, list items each on their
/// own), each under the last heading above it.
pub fn paragraphs(text: &str) -> Vec<Paragraph> {
    let mut out = Vec::new();
    let mut heading = String::new();
    let mut cur: Option<Paragraph> = None;
    let flush = |cur: &mut Option<Paragraph>, out: &mut Vec<Paragraph>| {
        if let Some(p) = cur.take()
            && !p.body.trim().is_empty()
        {
            out.push(p);
        }
    };
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix('#') {
            flush(&mut cur, &mut out);
            heading = h.trim_start_matches('#').trim().to_owned();
        } else if t.is_empty() || t.starts_with("- ") || t.starts_with("* ") {
            flush(&mut cur, &mut out);
            if !t.is_empty() {
                cur = Some(Paragraph {
                    heading: heading.clone(),
                    line: i + 1,
                    body: t.to_owned(),
                });
            }
        } else if let Some(p) = &mut cur {
            p.body.push('\n');
            p.body.push_str(t);
        } else {
            cur = Some(Paragraph {
                heading: heading.clone(),
                line: i + 1,
                body: t.to_owned(),
            });
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// An FTS5 query for the words of `query`: every word, as a prefix.
pub fn fts_query(query: &str) -> String {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `fast` model's request to file the inbox into the other files.
pub fn filing_request(model: &str, inbox: &str, files: &BTreeMap<String, String>) -> ModelRequest {
    let mut current = String::new();
    for (path, text) in files {
        current.push_str(&format!("=== {path}\n{text}\n"));
    }
    ModelRequest {
        model: model.to_owned(),
        system: "You keep a person's memory files for their browser's assistant. File each inbox entry into the right file: USER.md for who the person is and how they like things, people/<name>.md for people, sites/<host>.md for websites, projects/<name>.md for ongoing work, MEMORY.md for anything else. Merge duplicates and drop entries that are already known. Keep Markdown with short headings and one fact per list item. Answer with JSON only: {\"files\": {\"<path>\": \"<the whole new file>\"}} listing only files you change.".into(),
        messages: vec![Message::user(format!("Inbox:\n{inbox}\n\nCurrent files:\n{current}"))],
        tools: Vec::new(),
        effort: Effort::Auto,
        max_output_tokens: 8192,
        web_search: false,
    }
}

/// The files the model's answer changes: valid paths other than the inbox,
/// with text the filter lets through.
pub fn parse_filing(answer: &str) -> Result<BTreeMap<String, String>, String> {
    let start = answer.find('{').ok_or("no JSON in the answer")?;
    let end = answer.rfind('}').ok_or("no JSON in the answer")?;
    let v: serde_json::Value =
        serde_json::from_str(&answer[start..=end]).map_err(|e| e.to_string())?;
    let files = v["files"].as_object().ok_or("no files")?;
    let mut out = BTreeMap::new();
    for (path, text) in files {
        let Some(text) = text.as_str() else { continue };
        if !valid_path(path) || path == INBOX || refuse(text, &[]).is_some() {
            continue;
        }
        out.insert(path.clone(), text.to_owned());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_refused() {
        // 13 §13 check 7.
        assert_eq!(
            refuse("my card is 4111 1111 1111 1111", &[]),
            Some(Refused::CardNumber)
        );
        assert_eq!(
            refuse("card 4111-1111-1111-1111 exp 12/29", &[]),
            Some(Refused::CardNumber)
        );
        assert_eq!(
            refuse("order 4111 1111 1111 1112", &[]),
            None,
            "not Luhn-valid"
        );
        assert_eq!(
            refuse("key sk-abcdefghijklmnopqrstuv", &[]),
            Some(Refused::ApiKey)
        );
        assert_eq!(refuse("AKIAIOSFODNN7EXAMPLE", &[]), Some(Refused::ApiKey));
        assert_eq!(
            refuse("ghp_0123456789abcdefghij0123456789abcd", &[]),
            Some(Refused::ApiKey)
        );
        assert_eq!(
            refuse("token: aGVsbG8gd29ybGQgdGhpcyBpcyBsb25nZXIx", &[]),
            Some(Refused::ApiKey)
        );
        assert_eq!(
            refuse("my password is hunter22!", &["hunter22!".into()]),
            Some(Refused::Password)
        );
        assert_eq!(refuse("SSN 078-05-1120", &[]), Some(Refused::IdNumber));
        assert_eq!(
            refuse("IBAN DE89370400440532013000", &[]),
            Some(Refused::IdNumber)
        );
        assert_eq!(
            refuse("Likes window seats; phone +1 415 555 0100", &[]),
            None
        );
        assert_eq!(
            refuse("Prefers vegetarian restaurants near Market St", &[]),
            None
        );
    }

    #[test]
    fn utc_dates() {
        assert_eq!(utc(0), ("1970-01-01".into(), "1970-01-01 00:00".into()));
        assert_eq!(utc(1_795_000_000_000).1, "2026-11-18 11:06");
        assert_eq!(utc(951_782_400_000).0, "2000-02-29");
    }

    #[test]
    fn paths_stay_in_the_memory_folder() {
        for ok in [
            "USER.md",
            "MEMORY.md",
            "inbox.md",
            "people/ada.md",
            "sites/example.com.md",
            "projects/move-house.md",
        ] {
            assert!(valid_path(ok), "{ok}");
        }
        for bad in [
            "../x.md",
            "people/../../x.md",
            "people/.hidden.md",
            "notes.md",
            "people/a.txt",
            "/etc/x.md",
            "sites/a/b.md",
        ] {
            assert!(!valid_path(bad), "{bad}");
        }
    }

    #[test]
    fn paragraphs_carry_their_heading_and_line() {
        let p =
            paragraphs("# Food\n- Vegetarian\n- Likes tea\n\n## Travel\nWindow seats,\nalways.\n");
        assert_eq!(p.len(), 3);
        assert_eq!(
            p[1],
            Paragraph {
                heading: "Food".into(),
                line: 3,
                body: "- Likes tea".into()
            }
        );
        assert_eq!(
            p[2],
            Paragraph {
                heading: "Travel".into(),
                line: 6,
                body: "Window seats,\nalways.".into()
            }
        );
        assert_eq!(fts_query("window seat?"), "\"window\"* \"seat\"*");
        assert_eq!(
            inbox_entry("2026-11-18 09:00", "t1", "likes\n tea"),
            "- 2026-11-18 09:00 (task t1) likes tea\n"
        );
    }

    #[test]
    fn filing_keeps_only_valid_files() {
        let got = parse_filing("Here: {\"files\": {\"USER.md\": \"# Food\\n- Tea\", \"inbox.md\": \"\", \"../x.md\": \"x\", \"MEMORY.md\": \"card 4111111111111111\"}}").expect("parses");
        assert_eq!(got.keys().collect::<Vec<_>>(), ["USER.md"]);
        assert!(parse_filing("no").is_err());
        let r = filing_request(
            "m",
            "- tea",
            &BTreeMap::from([("USER.md".into(), "# Me".into())]),
        );
        assert!(format!("{:?}", r.messages[0]).contains("=== USER.md\\n# Me"));
    }
}
