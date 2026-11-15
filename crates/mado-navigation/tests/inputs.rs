//! Spec 05 §12 check 1: every row of `tests/nav/inputs.yaml` classifies
//! as written, in a fixed context.

use std::path::Path;

use mado_navigation::{Context, Input, classify};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    input: String,
    #[serde(default)]
    ctrl_enter: bool,
    expect: Input,
}

#[test]
fn the_classifier_table_holds() {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/nav/inputs.yaml"),
    )
    .expect("tests/nav/inputs.yaml");
    let rows: Vec<Row> = serde_norway::from_str(&text).expect("valid rows");
    assert!(rows.len() >= 300, "the table has {} rows", rows.len());
    let keywords = ["g", "w", "yt", "ddg"].map(String::from);
    let existing = [
        "/tmp",
        "/etc/hosts",
        "/Applications",
        "/Users/test/notes.txt",
        "/Users/test/Downloads",
        "/Users/test/My File.pdf",
    ];
    let ctx = Context {
        keywords: &keywords,
        visited: &|h| ["intranet", "nas", "router"].contains(&h),
        exists: &|p| existing.iter().any(|e| Path::new(e) == p),
        home: Some(Path::new("/Users/test")),
        now: "2026-11-14T12:00:00Z[UTC]".parse().expect("a time"),
    };
    let wrong: Vec<String> = rows
        .iter()
        .filter_map(|row| {
            let got = classify(&row.input, &ctx, row.ctrl_enter);
            (got != row.expect).then(|| {
                format!(
                    "{:?}{}: expected {:?}, got {got:?}",
                    row.input,
                    if row.ctrl_enter { " (⌘Enter)" } else { "" },
                    row.expect
                )
            })
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "{} rows differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
