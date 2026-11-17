//! Every command (spec 01 §4.2) with its parameters, read from the
//! TypeScript the `Command` enum generates, so the CLI's `call`,
//! `commands` and command groups follow the enum without a list of their
//! own (spec 14 §3).

const GENERATED: &str = include_str!("../../../ui/src/generated/Command.ts");

/// One command: its name and `field: type` parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub params: Vec<(String, String)>,
}

impl Entry {
    pub fn group(&self) -> &str {
        self.name.split('.').next().unwrap_or_default()
    }

    /// Whether some or all of its uses need Full control (06 §5, 14 §2.1):
    /// the families whose commands are protected, and settings changes
    /// (protected keys).
    pub fn may_need_full(&self) -> bool {
        matches!(
            self.group(),
            "vault" | "control" | "extension" | "profile" | "import"
        ) || matches!(
            self.name.as_str(),
            "settings.set"
                | "settings.reset"
                | "settings.import"
                | "ai.connection.add"
                | "ai.connection.update"
                | "ai.connection.remove"
                | "ai.role.set"
                | "ai.model.set"
                | "app.setDefaultBrowser"
                | "data.clear"
                | "history.delete"
        )
    }
}

/// Every command, in the enum's order.
pub fn all() -> Vec<Entry> {
    let mut out = Vec::new();
    for part in GENERATED.split("{ \"type\": \"").skip(1) {
        let Some((name, rest)) = part.split_once('"') else {
            continue;
        };
        let body = rest.split('}').next().unwrap_or_default();
        let params = fields(body);
        if !out.iter().any(|e: &Entry| e.name == name) {
            out.push(Entry {
                name: name.to_owned(),
                params,
            });
        }
    }
    out
}

/// `, a: T, b: U | null, ` into pairs; nested types keep their commas.
fn fields(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in body.chars() {
        match c {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if c == ',' && depth == 0 {
            push(&mut out, &current);
            current.clear();
        } else {
            current.push(c);
        }
    }
    push(&mut out, &current);
    out
}

fn push(out: &mut Vec<(String, String)>, field: &str) {
    if let Some((k, v)) = field.split_once(':') {
        let k = k.trim().trim_matches('"');
        if !k.is_empty() {
            out.push((k.to_owned(), v.trim().to_owned()));
        }
    }
}

/// The CLI group a command belongs to: `password` for `vault`.
pub fn cli_group(family: &str) -> &str {
    if family == "vault" {
        "password"
    } else {
        family
    }
}

/// The command a group verb names: `password reveal` is `vault.get`.
pub fn resolve(group: &str, verb: &str) -> String {
    match (group, verb) {
        ("password", "reveal") => "vault.get".into(),
        ("password", v) => format!("vault.{v}"),
        (g, v) => format!("{g}.{v}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_of_the_enum_is_listed_with_its_parameters() {
        let all = all();
        assert!(all.len() > 150, "{}", all.len());
        let set = all
            .iter()
            .find(|e| e.name == "settings.set")
            .expect("settings.set");
        assert_eq!(
            set.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(),
            ["key", "value", "token"]
        );
        let tab = all.iter().find(|e| e.name == "tab.new").expect("tab.new");
        assert!(
            tab.params
                .iter()
                .any(|(k, t)| k == "position" && t.contains("null"))
        );
        assert!(
            all.iter()
                .any(|e| e.name == "state.get" && e.params.is_empty())
        );
        assert_eq!(resolve("password", "reveal"), "vault.get");
        assert_eq!(cli_group("vault"), "password");
        assert!(
            all.iter()
                .find(|e| e.name == "vault.get")
                .is_some_and(Entry::may_need_full)
        );
    }
}
