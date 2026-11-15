//! Spec 05 §12 check 3: Chromium's IDN spoof cases show as punycode, and
//! its safe cases as Unicode, against Chromium's IDN test top domains.

use std::path::Path;

use mado_navigation::{TopDomains, host_for_display};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    ascii: String,
    unicode: String,
    result: String,
}

fn read(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/nav")
            .join(name),
    )
    .expect("test data")
}

#[test]
fn spoofs_stay_in_punycode() {
    let cases: Vec<Case> = serde_norway::from_str(&read("idn-spoof.yaml")).expect("cases");
    let top = TopDomains::from_list(&read("idn-test-top-domains.txt"));
    let mut wrong = Vec::new();
    for c in &cases {
        let (shown, _) = host_for_display(&c.ascii, &top);
        let expected = if c.result == "safe" {
            &c.unicode
        } else {
            &c.ascii
        };
        if &shown != expected {
            wrong.push(format!(
                "{} ({}): expected {expected}, got {shown}",
                c.ascii, c.result
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cases differ:\n{}",
        wrong.len(),
        cases.len(),
        wrong.join("\n")
    );
}
