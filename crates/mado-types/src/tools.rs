//! The agent tools (spec 13 §8): names, descriptions and JSON Schemas, as
//! models, MCP clients and `native_actions` see them, and the snapshot
//! diff that actions answer with.

use serde_json::{Value, json};

/// One tool.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    /// Only reads (the `read` control scope); the rest need `act`.
    pub read_only: bool,
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

fn tab() -> Value {
    json!({ "type": "string", "description": "A tab ID from open or native_context" })
}

/// Every tool of 13 §8.
pub fn all() -> Vec<Tool> {
    let t = |name, description, parameters, read_only| Tool {
        name,
        description,
        parameters,
        read_only,
    };
    let point = json!({ "type": "number" });
    vec![
        t(
            "open",
            "Open a URL in a new agent tab; answers the tab and its snapshot.",
            obj(json!({ "url": { "type": "string" } }), &["url"]),
            false,
        ),
        t(
            "navigate",
            "Go to a URL in a tab, or back, forward or reload; answers the new snapshot.",
            obj(
                json!({ "tab": tab(), "url": { "type": "string", "description": "A URL, or back, forward or reload" } }),
                &["tab", "url"],
            ),
            false,
        ),
        t(
            "snapshot",
            "The page as an accessibility tree: one line per element with a ref such as e12. interactive keeps only what can be clicked or typed in; ref limits it to one element.",
            obj(
                json!({ "tab": tab(), "interactive": { "type": "boolean" }, "ref": { "type": "string" } }),
                &["tab"],
            ),
            true,
        ),
        t(
            "text",
            "The page's readable text.",
            obj(json!({ "tab": tab() }), &["tab"]),
            true,
        ),
        t(
            "screenshot",
            "A PNG of the visible page.",
            obj(
                json!({ "tab": tab(), "ref": { "type": "string" } }),
                &["tab"],
            ),
            true,
        ),
        t(
            "click",
            "Click an element by ref (or a point) as a person would; answers what changed on the page, or the element that covers it.",
            obj(
                json!({ "tab": tab(), "ref": { "type": "string" }, "x": point, "y": point }),
                &["tab"],
            ),
            false,
        ),
        t(
            "type",
            "Type text into an element by ref (or the focused one); submit presses Enter after.",
            obj(
                json!({ "tab": tab(), "ref": { "type": "string" }, "text": { "type": "string" }, "submit": { "type": "boolean" } }),
                &["tab", "text"],
            ),
            false,
        ),
        t(
            "choose",
            "Choose an option of a select element by its value or text.",
            obj(
                json!({ "tab": tab(), "ref": { "type": "string" }, "value": { "type": "string" } }),
                &["tab", "ref", "value"],
            ),
            false,
        ),
        t(
            "press",
            "Press keys such as Enter, Tab, Escape or Cmd+A.",
            obj(
                json!({ "tab": tab(), "keys": { "type": "string" } }),
                &["tab", "keys"],
            ),
            false,
        ),
        t(
            "hover",
            "Move the pointer over an element by ref (or a point).",
            obj(
                json!({ "tab": tab(), "ref": { "type": "string" }, "x": point, "y": point }),
                &["tab"],
            ),
            false,
        ),
        t(
            "scroll",
            "Scroll the page or an element by ref; without dx and dy, scrolls the element into view.",
            obj(
                json!({ "tab": tab(), "ref": { "type": "string" }, "dx": point, "dy": point }),
                &["tab"],
            ),
            false,
        ),
        t(
            "drag",
            "Drag from one element to another by ref.",
            obj(
                json!({ "tab": tab(), "from": { "type": "string" }, "to": { "type": "string" } }),
                &["tab", "from", "to"],
            ),
            false,
        ),
        t(
            "wait",
            "Wait until text or a ref appears, or the page settles; answers the snapshot.",
            obj(
                json!({ "tab": tab(), "for": { "type": "string" }, "timeoutMs": { "type": "integer" } }),
                &["tab"],
            ),
            true,
        ),
        t(
            "run",
            "Run JavaScript that calls these tools as functions (await click({tab, ref})), at most 200 steps, in a sandbox inside Mado, never in the page.",
            obj(json!({ "code": { "type": "string" } }), &["code"]),
            false,
        ),
        t(
            "fetch",
            "GET a URL with the profile's cookies; text only, 2 MB at most.",
            obj(json!({ "url": { "type": "string" } }), &["url"]),
            true,
        ),
        t(
            "handoff",
            "Hand the tab to the person (a sign-in, a CAPTCHA, a payment) and wait until they hand it back.",
            obj(
                json!({ "tab": tab(), "reason": { "type": "string" } }),
                &["tab", "reason"],
            ),
            false,
        ),
        t(
            "ask",
            "Ask the person a question; answers what they said.",
            obj(
                json!({ "question": { "type": "string" }, "options": { "type": "array", "items": { "type": "string" } } }),
                &["question"],
            ),
            false,
        ),
        t(
            "sign_in",
            "Fill the tab's sign-in form from the person's passwords; answers whether it filled and the username, never the password.",
            obj(json!({ "tab": tab() }), &["tab"]),
            false,
        ),
        t(
            "autofill",
            "Fill a form with the person's contact, address or card; a card asks for Touch ID.",
            obj(
                json!({ "tab": tab(), "what": { "type": "string", "enum": ["form", "address", "card"] } }),
                &["tab", "what"],
            ),
            false,
        ),
        t(
            "upload",
            "Choose files for a file input by ref (needs approval).",
            obj(
                json!({ "tab": tab(), "ref": { "type": "string" }, "files": { "type": "array", "items": { "type": "string" } } }),
                &["tab", "ref", "files"],
            ),
            false,
        ),
        t(
            "download",
            "Download a link by ref or a URL into the task folder.",
            obj(
                json!({ "tab": tab(), "ref": { "type": "string" }, "url": { "type": "string" } }),
                &["tab"],
            ),
            false,
        ),
        t(
            "history",
            "Visits matching words or a host.",
            obj(
                json!({ "query": { "type": "string" }, "host": { "type": "string" } }),
                &[],
            ),
            true,
        ),
        t(
            "bookmarks",
            "Bookmarks matching words.",
            obj(json!({ "query": { "type": "string" } }), &[]),
            true,
        ),
        t(
            "bookmark_add",
            "Bookmark a URL.",
            obj(
                json!({ "url": { "type": "string" }, "title": { "type": "string" } }),
                &["url"],
            ),
            false,
        ),
        t(
            "memory_search",
            "Search what Mado remembers for the person.",
            obj(json!({ "query": { "type": "string" } }), &["query"]),
            true,
        ),
        t(
            "memory_read",
            "Read one memory file.",
            obj(json!({ "path": { "type": "string" } }), &["path"]),
            true,
        ),
        t(
            "remember",
            "Remember a fact for later tasks.",
            obj(json!({ "text": { "type": "string" } }), &["text"]),
            false,
        ),
        t("skills", "The skills Mado has.", obj(json!({}), &[]), true),
        t(
            "skill",
            "Read one skill.",
            obj(json!({ "name": { "type": "string" } }), &["name"]),
            true,
        ),
        t(
            "skill_save",
            "Save how to do something as a skill: a SKILL.md document with front matter (name, description, optional sites) and Markdown steps. The person approves it first.",
            obj(
                json!({ "name": { "type": "string" }, "document": { "type": "string" }, "replacing": { "type": "boolean" } }),
                &["name", "document"],
            ),
            false,
        ),
        t(
            "native_context",
            "Mado's windows, tabs, groups, a settings summary and features.",
            obj(json!({}), &[]),
            true,
        ),
        t(
            "native_actions",
            "Mado's own actions (not protected ones) with their parameters.",
            obj(json!({}), &[]),
            true,
        ),
        t(
            "native_apply",
            "Run one of Mado's own actions; answers its result and an undo token when it can be undone.",
            obj(
                json!({ "action": { "type": "string" }, "args": { "type": "object" } }),
                &["action"],
            ),
            false,
        ),
        t(
            "native_undo",
            "Undo an action by its token.",
            obj(json!({ "token": { "type": "string" } }), &["token"]),
            false,
        ),
        t(
            "delegate",
            "Give a sub-task to a helper agent; answers its result.",
            obj(
                json!({ "prompt": { "type": "string" }, "scope": { "type": "array", "items": { "type": "string" } } }),
                &["prompt"],
            ),
            false,
        ),
        t(
            "close",
            "Close an agent tab.",
            obj(json!({ "tab": tab() }), &["tab"]),
            false,
        ),
    ]
}

pub fn find(name: &str) -> Option<Tool> {
    all().into_iter().find(|t| t.name == name)
}

/// What changed between two snapshots: lines that came and went, in
/// order, at most 200 of each.
pub fn diff(before: &[String], after: &[String]) -> Value {
    let strip = |l: &String| l.trim_start().to_owned();
    let old: std::collections::BTreeSet<String> = before.iter().map(strip).collect();
    let new: std::collections::BTreeSet<String> = after.iter().map(strip).collect();
    let added: Vec<&String> = after
        .iter()
        .filter(|l| !old.contains(&strip(l)))
        .take(200)
        .collect();
    let removed: Vec<&String> = before
        .iter()
        .filter(|l| !new.contains(&strip(l)))
        .take(200)
        .collect();
    json!({ "added": added, "removed": removed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_of_the_spec_has_an_object_schema() {
        let all = all();
        assert_eq!(all.len(), 36);
        for t in &all {
            assert_eq!(t.parameters["type"], "object", "{}", t.name);
            for r in t.parameters["required"].as_array().into_iter().flatten() {
                assert!(
                    t.parameters["properties"]
                        .get(r.as_str().unwrap_or_default())
                        .is_some(),
                    "{}: {r}",
                    t.name
                );
            }
        }
        assert!(find("snapshot").is_some_and(|t| t.read_only));
        assert!(find("click").is_some_and(|t| !t.read_only));
    }

    #[test]
    fn diffs_name_what_came_and_went() {
        let a = vec![
            "e1 button \"Buy\"".to_owned(),
            "e2 text \"0 items\"".to_owned(),
        ];
        let b = vec![
            "e1 button \"Buy\"".to_owned(),
            "  e3 text \"1 item\"".to_owned(),
        ];
        assert_eq!(
            diff(&a, &b),
            json!({ "added": ["  e3 text \"1 item\""], "removed": ["e2 text \"0 items\""] })
        );
    }
}
