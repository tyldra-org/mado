//! Skills (spec 13 §11): folders with a `SKILL.md`, YAML front matter
//! (`name`, `description`, optional `sites`) and Markdown, the format
//! Claude Code reads. Built-in skills ship in the app; the person's are in
//! `<profile>/AI/skills/<name>/SKILL.md`.

use std::path::Path;

/// One skill.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub sites: Vec<String>,
    #[serde(skip)]
    pub document: String,
    pub builtin: bool,
}

/// The skills that ship with Mado.
const BUILTIN: [&str; 2] = [
    include_str!("../skills/research/SKILL.md"),
    include_str!("../skills/compare-prices/SKILL.md"),
];

/// A skill's name: lowercase letters, digits and hyphens, at most 64.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn unquote(v: &str) -> String {
    v.trim().trim_matches(|c| c == '"' || c == '\'').to_owned()
}

/// Reads `SKILL.md`; the front matter needs a valid `name` and a
/// `description`.
pub fn parse(document: &str) -> Result<Skill, String> {
    let rest = document
        .strip_prefix("---\n")
        .ok_or("a skill starts with --- and its front matter")?;
    let end = rest
        .find("\n---")
        .ok_or("the front matter has no closing ---")?;
    let (mut name, mut description, mut sites) = (String::new(), String::new(), Vec::new());
    let mut in_sites = false;
    for line in rest[..end].lines() {
        if in_sites && let Some(item) = line.trim().strip_prefix("- ") {
            sites.push(unquote(item).to_lowercase());
            continue;
        }
        in_sites = false;
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "name" => name = unquote(value),
            "description" => description = unquote(value),
            "sites" => {
                let v = value.trim();
                if let Some(list) = v.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                    sites = list
                        .split(',')
                        .map(|s| unquote(s).to_lowercase())
                        .filter(|s| !s.is_empty())
                        .collect();
                } else if v.is_empty() {
                    in_sites = true;
                }
            }
            _ => {}
        }
    }
    if !valid_name(&name) {
        return Err("the name must be lowercase letters, digits and hyphens".into());
    }
    if description.is_empty() {
        return Err("a skill needs a description".into());
    }
    Ok(Skill {
        name,
        description,
        sites,
        document: document.to_owned(),
        builtin: false,
    })
}

/// The built-in skills.
pub fn builtins() -> Vec<Skill> {
    BUILTIN
        .iter()
        .filter_map(|d| parse(d).ok())
        .map(|s| Skill { builtin: true, ..s })
        .collect()
}

/// The person's skills in `dir`, then the built-in ones they do not
/// replace.
pub fn all(dir: &Path) -> Vec<Skill> {
    let mut out: Vec<Skill> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| std::fs::read_to_string(e.path().join("SKILL.md")).ok())
                .filter_map(|d| parse(&d).ok())
                .collect()
        })
        .unwrap_or_default();
    for b in builtins() {
        if !out.iter().any(|s| s.name == b.name) {
            out.push(b);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The prompt's index: one line per skill.
pub fn index(skills: &[Skill]) -> String {
    skills
        .iter()
        .map(|s| format!("- {}: {}\n", s.name, s.description))
        .collect()
}

/// The skill whose `sites` list `host` or a parent of it.
pub fn for_host<'a>(skills: &'a [Skill], host: &str) -> Option<&'a Skill> {
    let host = host.to_lowercase();
    skills.iter().find(|s| {
        s.sites
            .iter()
            .any(|site| host == *site || host.ends_with(&format!(".{site}")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "---\nname: book-a-table\ndescription: Book a table at a restaurant\nsites: [opentable.com, \"resy.com\"]\n---\n# Steps\n1. Search.\n";

    #[test]
    fn a_skill_reads_its_front_matter() {
        let s = parse(DOC).expect("parses");
        assert_eq!(s.name, "book-a-table");
        assert_eq!(s.sites, ["opentable.com", "resy.com"]);
        let list = parse("---\nname: x\ndescription: d\nsites:\n  - a.com\n  - b.org\n---\nbody")
            .expect("list");
        assert_eq!(list.sites, ["a.com", "b.org"]);
        assert!(parse("---\nname: Bad Name\ndescription: d\n---\n").is_err());
        assert!(parse("---\nname: ok\n---\n").is_err());
        assert!(parse("no front matter").is_err());
    }

    #[test]
    fn sites_match_subdomains_and_the_index_lists_every_skill() {
        let s = vec![parse(DOC).expect("parses")];
        assert_eq!(
            for_host(&s, "www.opentable.com").map(|s| s.name.as_str()),
            Some("book-a-table")
        );
        assert!(for_host(&s, "notopentable.com").is_none());
        assert_eq!(index(&s), "- book-a-table: Book a table at a restaurant\n");
    }

    #[test]
    fn the_person_s_skills_replace_built_in_ones() {
        let dir = tempfile::tempdir().expect("dir");
        assert_eq!(builtins().len(), BUILTIN.len());
        let mine = dir.path().join("research");
        std::fs::create_dir_all(&mine).expect("mkdir");
        std::fs::write(
            mine.join("SKILL.md"),
            "---\nname: research\ndescription: Mine\n---\n",
        )
        .expect("write");
        let all = all(dir.path());
        let r = all.iter().find(|s| s.name == "research").expect("research");
        assert_eq!(r.description, "Mine");
        assert!(!r.builtin);
        assert!(all.iter().any(|s| s.name == "compare-prices" && s.builtin));
    }
}
