//! `mado setup` and `mado remove` (spec 14 §5): Mado's MCP server and
//! guide in other agents, and `mado` on the PATH. Setup records each file's
//! original bytes and the hash of what it wrote in `<root>/setup.json`;
//! remove restores the original when the file still hashes to what setup
//! wrote, and otherwise takes out only Mado's entries.

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use data_encoding::{BASE64, HEXLOWER};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Every target, in the order `mado setup` lists them.
pub const TARGETS: [&str; 4] = ["claude", "codex", "cursor", "path"];

/// The line setup adds to a shell profile.
const PROFILE_MARK: &str = "# added by mado setup";

/// Where setup reads and writes.
#[derive(Debug, Clone)]
pub struct Env {
    pub home: PathBuf,
    /// Mado's data root, where `setup.json` lives.
    pub root: PathBuf,
    /// This `mado` executable.
    pub mado: PathBuf,
    /// Folders searched for `claude`.
    pub path: Vec<PathBuf>,
    /// Answer yes to "add to your shell profile?".
    pub yes: bool,
}

impl Env {
    pub fn system(root: PathBuf, yes: bool) -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let mado = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .unwrap_or_else(|_| "mado".into());
        let mut path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        path.extend([
            home.join(".local/bin"),
            "/opt/homebrew/bin".into(),
            "/usr/local/bin".into(),
        ]);
        Self {
            home,
            root,
            mado,
            path,
            yes,
        }
    }

    fn record_file(&self) -> PathBuf {
        self.root.join("setup.json")
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        self.path.iter().map(|d| d.join(name)).find(|p| p.is_file())
    }

    fn mado_str(&self) -> String {
        self.mado.to_string_lossy().into_owned()
    }
}

fn sha(bytes: &[u8]) -> String {
    HEXLOWER.encode(&Sha256::digest(bytes))
}

fn load(env: &Env) -> Value {
    std::fs::read(env.record_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}))
}

fn save(env: &Env, record: &Value) -> Result<(), String> {
    std::fs::create_dir_all(&env.root).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(record).map_err(|e| e.to_string())?;
    std::fs::write(env.record_file(), text).map_err(|e| e.to_string())
}

/// Writes `bytes` to `path`, keeping the original the first time.
fn write_file(entry: &mut Value, path: &Path, bytes: &[u8]) -> Result<(), String> {
    let key = path.to_string_lossy().into_owned();
    if entry["files"].get(&key).is_none() {
        let before = std::fs::read(path).ok().map(|b| BASE64.encode(&b));
        entry["files"][&key] = json!({ "before": before });
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    entry["files"][&key]["written"] = json!(sha(bytes));
    Ok(())
}

// Each target's own entries.

fn codex_add(text: &str, mado: &str) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("config.toml: {e}"))?;
    if !doc.contains_key("mcp_servers") {
        let mut t = toml_edit::Table::new();
        t.set_implicit(true);
        doc.insert("mcp_servers", toml_edit::Item::Table(t));
    }
    let servers = doc["mcp_servers"]
        .as_table_mut()
        .ok_or("config.toml: mcp_servers is not a table")?;
    let mut mado_table = toml_edit::Table::new();
    mado_table.insert("command", toml_edit::value(mado));
    let mut args = toml_edit::Array::new();
    args.push("mcp");
    mado_table.insert("args", toml_edit::value(args));
    servers.insert("mado", toml_edit::Item::Table(mado_table));
    Ok(doc.to_string())
}

fn codex_strip(text: &str) -> Option<String> {
    let mut doc: toml_edit::DocumentMut = text.parse().ok()?;
    let servers = doc.get_mut("mcp_servers")?.as_table_mut()?;
    servers.remove("mado");
    if servers.is_empty() {
        doc.remove("mcp_servers");
    }
    Some(doc.to_string())
}

fn cursor_add(text: &str, mado: &str) -> Result<String, String> {
    let mut v: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(text).map_err(|e| format!("mcp.json: {e}"))?
    };
    if !v.is_object() {
        return Err("mcp.json is not an object".into());
    }
    if !v["mcpServers"].is_object() {
        v["mcpServers"] = json!({});
    }
    v["mcpServers"]["mado"] = json!({ "command": mado, "args": ["mcp"] });
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&v).unwrap_or_default()
    ))
}

fn cursor_strip(text: &str) -> Option<String> {
    let mut v: Value = serde_json::from_str(text).ok()?;
    let servers = v.get_mut("mcpServers")?.as_object_mut()?;
    servers.remove("mado");
    if servers.is_empty()
        && let Some(o) = v.as_object_mut()
    {
        o.remove("mcpServers");
    }
    Some(format!(
        "{}\n",
        serde_json::to_string_pretty(&v).unwrap_or_default()
    ))
}

fn profile_strip(text: &str) -> String {
    text.lines()
        .filter(|l| !l.ends_with(PROFILE_MARK))
        .map(|l| format!("{l}\n"))
        .collect()
}

fn skill_document() -> String {
    format!(
        "---\nname: mado\ndescription: Control the Mado web browser from the terminal with the mado command: open pages, run browser tasks, read tabs and change Mado's settings.\n---\n{}",
        crate::guide::GUIDE
    )
}

fn shell_profile(env: &Env) -> PathBuf {
    let shell = std::env::var("SHELL").unwrap_or_default();
    if shell.ends_with("bash") {
        env.home.join(".bash_profile")
    } else {
        env.home.join(".zshrc")
    }
}

fn ask(question: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("{question} [y/N] ");
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).is_ok() && answer.trim().eq_ignore_ascii_case("y")
}

fn run(program: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} {}: {}",
            program.display(),
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Sets up one target; returns what it did, for the person.
pub fn setup(env: &Env, target: &str) -> Result<String, String> {
    let mut record = load(env);
    let mut entry = record
        .get(target)
        .cloned()
        .unwrap_or_else(|| json!({ "files": {} }));
    let mado = env.mado_str();
    let said = match target {
        "codex" => {
            let file = env.home.join(".codex/config.toml");
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            write_file(&mut entry, &file, codex_add(&text, &mado)?.as_bytes())?;
            format!("Added Mado's MCP server to {}.", file.display())
        }
        "cursor" => {
            let file = env.home.join(".cursor/mcp.json");
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            write_file(&mut entry, &file, cursor_add(&text, &mado)?.as_bytes())?;
            format!("Added Mado's MCP server to {}.", file.display())
        }
        "claude" => {
            let claude = env
                .which("claude")
                .ok_or("Claude Code is not installed (claude was not found).")?;
            // Replace an earlier entry so a moved Mado is found.
            let _ = run(&claude, &["mcp", "remove", "--scope", "user", "mado"]);
            run(
                &claude,
                &["mcp", "add", "--scope", "user", "mado", "--", &mado, "mcp"],
            )?;
            entry["claudeMcp"] = json!(claude);
            let skill = env.home.join(".claude/skills/mado/SKILL.md");
            write_file(&mut entry, &skill, skill_document().as_bytes())?;
            format!(
                "Added Mado's MCP server to Claude Code and its guide to {}.",
                skill.display()
            )
        }
        "path" => {
            let bin = env.home.join(".local/bin");
            let link = bin.join("mado");
            match std::fs::read_link(&link) {
                Ok(t) if t == env.mado => {}
                Ok(_) => {
                    std::fs::remove_file(&link).map_err(|e| e.to_string())?;
                }
                Err(_) if link.exists() => {
                    return Err(format!("{} exists and is not Mado's link.", link.display()));
                }
                Err(_) => {}
            }
            std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
            if std::fs::read_link(&link).is_err() {
                std::os::unix::fs::symlink(&env.mado, &link).map_err(|e| e.to_string())?;
            }
            entry["link"] = json!(link);
            let on_path = std::env::var_os("PATH")
                .is_some_and(|p| std::env::split_paths(&p).any(|d| d == bin));
            let profile = shell_profile(env);
            let current = std::fs::read_to_string(&profile).unwrap_or_default();
            let mut said = format!("Linked {} to {}.", link.display(), env.mado.display());
            if !on_path && !current.contains(PROFILE_MARK) {
                if env.yes
                    || ask(&format!(
                        "Add ~/.local/bin to your PATH in {}?",
                        profile.display()
                    ))
                {
                    let line = format!("export PATH=\"$HOME/.local/bin:$PATH\"  {PROFILE_MARK}\n");
                    let sep = if current.is_empty() || current.ends_with('\n') {
                        ""
                    } else {
                        "\n"
                    };
                    write_file(
                        &mut entry,
                        &profile,
                        format!("{current}{sep}{line}").as_bytes(),
                    )?;
                    said.push_str(&format!(
                        " Added ~/.local/bin to your PATH in {}; open a new terminal.",
                        profile.display()
                    ));
                } else {
                    said.push_str(
                        " ~/.local/bin is not on your PATH; add it to use mado everywhere.",
                    );
                }
            }
            said
        }
        _ => {
            return Err(format!(
                "unknown target {target}; use one of {}",
                TARGETS.join(", ")
            ));
        }
    };
    record[target] = entry;
    save(env, &record)?;
    Ok(said)
}

/// Undoes one target's setup.
pub fn remove(env: &Env, target: &str) -> Result<String, String> {
    let mut record = load(env);
    let Some(entry) = record.get(target).cloned() else {
        return Ok(format!("{target} was not set up."));
    };
    let mut kept = Vec::new();
    for (path, f) in entry["files"].as_object().into_iter().flatten() {
        let path = PathBuf::from(path);
        let Ok(now) = std::fs::read(&path) else {
            continue;
        };
        let before = f["before"]
            .as_str()
            .and_then(|b| BASE64.decode(b.as_bytes()).ok());
        if f["written"].as_str() == Some(sha(&now).as_str()) {
            // Untouched since setup: the original bytes, exactly.
            match before {
                Some(b) => std::fs::write(&path, b).map_err(|e| e.to_string())?,
                None => {
                    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::remove_dir(dir);
                    }
                }
            }
            continue;
        }
        // Changed since: only Mado's entries go.
        let text = String::from_utf8_lossy(&now);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let stripped = match name {
            "config.toml" => codex_strip(&text),
            "mcp.json" => cursor_strip(&text),
            "SKILL.md" => None,
            _ => Some(profile_strip(&text)),
        };
        match stripped {
            Some(t) => std::fs::write(&path, t).map_err(|e| e.to_string())?,
            None if name == "SKILL.md" => {
                std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                if let Some(dir) = path.parent() {
                    let _ = std::fs::remove_dir(dir);
                }
            }
            None => kept.push(path.display().to_string()),
        }
    }
    if let Some(claude) = entry["claudeMcp"].as_str() {
        let _ = run(
            Path::new(claude),
            &["mcp", "remove", "--scope", "user", "mado"],
        );
    }
    if let Some(link) = entry["link"].as_str() {
        let link = Path::new(link);
        if std::fs::read_link(link).is_ok() {
            std::fs::remove_file(link).map_err(|e| e.to_string())?;
        }
    }
    if let Some(o) = record.as_object_mut() {
        o.remove(target);
    }
    save(env, &record)?;
    if kept.is_empty() {
        Ok(format!("Removed Mado from {target}."))
    } else {
        Ok(format!(
            "Removed Mado from {target}; could not read {} to take Mado out.",
            kept.join(", ")
        ))
    }
}

/// Each target: set up or not, and whether its tool is there.
pub fn status(env: &Env) -> Value {
    let record = load(env);
    Value::Array(
        TARGETS
            .iter()
            .map(|t| {
                let available = match *t {
                    "claude" => env.which("claude").is_some(),
                    "codex" => env.which("codex").is_some() || env.home.join(".codex").is_dir(),
                    "cursor" => {
                        env.home.join(".cursor").is_dir()
                            || Path::new("/Applications/Cursor.app").is_dir()
                    }
                    _ => true,
                };
                json!({ "target": t, "setUp": record.get(*t).is_some(), "available": available })
            })
            .collect(),
    )
}

/// The MCP configuration other clients take (Settings' Copy config).
pub fn config(env: &Env) -> Value {
    json!({ "mcpServers": { "mado": { "command": env.mado_str(), "args": ["mcp"] } } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(dir: &Path) -> Env {
        Env {
            home: dir.join("home"),
            root: dir.join("root"),
            mado: "/Applications/Mado.app/Contents/MacOS/mado".into(),
            path: vec![dir.join("bin")],
            yes: true,
        }
    }

    const CODEX: &str =
        "# my settings\nmodel = \"gpt-5\"   # keep this\n\n[mcp_servers.other]\ncommand = \"x\"\n";

    #[test]
    fn codex_setup_then_remove_is_byte_identical() {
        // 14 §6 check 5.
        let dir = tempfile::tempdir().expect("dir");
        let e = env(dir.path());
        let file = e.home.join(".codex/config.toml");
        std::fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
        std::fs::write(&file, CODEX).expect("write");
        setup(&e, "codex").expect("setup");
        let written = std::fs::read_to_string(&file).expect("read");
        assert!(
            written.starts_with(CODEX),
            "keeps the person's lines: {written}"
        );
        assert!(written.contains("[mcp_servers.mado]\ncommand = \"/Applications/Mado.app/Contents/MacOS/mado\"\nargs = [\"mcp\"]"), "{written}");
        // Twice is the same as once, and remove still restores the original.
        setup(&e, "codex").expect("again");
        remove(&e, "codex").expect("remove");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), CODEX);
        assert_eq!(load(&e), json!({}));
    }

    #[test]
    fn a_file_changed_after_setup_loses_only_mado() {
        let dir = tempfile::tempdir().expect("dir");
        let e = env(dir.path());
        let file = e.home.join(".codex/config.toml");
        std::fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
        std::fs::write(&file, CODEX).expect("write");
        setup(&e, "codex").expect("setup");
        let mut t = std::fs::read_to_string(&file).expect("read");
        t.push_str("\n[profiles.fast]\nmodel = \"o4\"\n");
        std::fs::write(&file, &t).expect("edit");
        remove(&e, "codex").expect("remove");
        let after = std::fs::read_to_string(&file).expect("read");
        assert!(!after.contains("mcp_servers.mado"), "{after}");
        assert!(
            after.contains("[mcp_servers.other]")
                && after.contains("[profiles.fast]")
                && after.contains("# keep this")
        );
    }

    #[test]
    fn cursor_without_a_file_creates_and_deletes_it() {
        let dir = tempfile::tempdir().expect("dir");
        let e = env(dir.path());
        setup(&e, "cursor").expect("setup");
        let file = e.home.join(".cursor/mcp.json");
        let v: Value = serde_json::from_slice(&std::fs::read(&file).expect("read")).expect("json");
        assert_eq!(v["mcpServers"]["mado"]["args"], json!(["mcp"]));
        remove(&e, "cursor").expect("remove");
        assert!(!file.exists());
        // With other servers, a changed file keeps them.
        std::fs::create_dir_all(file.parent().expect("dir")).expect("mkdir");
        std::fs::write(&file, "{\"mcpServers\":{\"other\":{\"command\":\"x\"}}}").expect("write");
        setup(&e, "cursor").expect("setup");
        let mut v: Value =
            serde_json::from_slice(&std::fs::read(&file).expect("read")).expect("json");
        v["theme"] = json!("dark");
        std::fs::write(&file, v.to_string()).expect("edit");
        remove(&e, "cursor").expect("remove");
        let v: Value = serde_json::from_slice(&std::fs::read(&file).expect("read")).expect("json");
        assert_eq!(
            v,
            json!({ "mcpServers": { "other": { "command": "x" } }, "theme": "dark" })
        );
    }

    #[test]
    fn claude_runs_its_cli_and_writes_the_guide() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("dir");
        let e = env(dir.path());
        assert!(setup(&e, "claude").is_err(), "no claude");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("mkdir");
        let log = dir.path().join("claude.log");
        std::fs::write(
            bin.join("claude"),
            format!("#!/bin/sh\necho \"$@\" >> {}\n", log.display()),
        )
        .expect("write");
        std::fs::set_permissions(bin.join("claude"), std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
        setup(&e, "claude").expect("setup");
        let skill = e.home.join(".claude/skills/mado/SKILL.md");
        assert!(
            std::fs::read_to_string(&skill)
                .expect("skill")
                .starts_with("---\nname: mado\n")
        );
        remove(&e, "claude").expect("remove");
        assert!(!skill.exists());
        let calls = std::fs::read_to_string(&log).expect("log");
        assert!(
            calls.contains(
                "mcp add --scope user mado -- /Applications/Mado.app/Contents/MacOS/mado mcp"
            ),
            "{calls}"
        );
        assert!(
            calls.trim_end().ends_with("mcp remove --scope user mado"),
            "{calls}"
        );
    }

    #[test]
    fn path_links_mado_and_restores_the_profile() {
        let dir = tempfile::tempdir().expect("dir");
        let e = env(dir.path());
        let profile = shell_profile(&e);
        std::fs::create_dir_all(&e.home).expect("mkdir");
        std::fs::write(&profile, "alias ll='ls -l'").expect("write");
        setup(&e, "path").expect("setup");
        let link = e.home.join(".local/bin/mado");
        assert_eq!(std::fs::read_link(&link).expect("link"), e.mado);
        assert!(
            std::fs::read_to_string(&profile)
                .expect("read")
                .contains(PROFILE_MARK)
        );
        remove(&e, "path").expect("remove");
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(
            std::fs::read_to_string(&profile).expect("read"),
            "alias ll='ls -l'"
        );
        assert_eq!(
            status(&e)[3],
            json!({ "target": "path", "setUp": false, "available": true })
        );
    }
}
