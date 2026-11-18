//! Memory and skills in the core (spec 13 §10, §11): the memory files and
//! their search index, `remember` with its filter, the daily filing and
//! its undo; skills from the app and the profile, their index for the task
//! prompt, site hints and saving with approval.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use mado_agent::memory::{self, INBOX};
use mado_agent::skills::{self, Skill};
use mado_types::command::{Command, Origin};
use mado_types::patch::PatchRecorder;
use mado_types::{ErrorCode, MadoError, TaskId};
use rusqlite::params;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::{Core, Reply};

/// The filing runs this often (13 §10).
pub(super) const FILING_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

fn err(code: ErrorCode, key: &str) -> MadoError {
    MadoError::new(code, key.to_owned())
}

fn off() -> MadoError {
    err(ErrorCode::Unsupported, "error.ai.memoryOff")
}

fn bad_path() -> MadoError {
    err(ErrorCode::InvalidInput, "error.ai.memoryPath")
}

fn io(e: &std::io::Error) -> MadoError {
    if e.kind() == std::io::ErrorKind::NotFound {
        return err(ErrorCode::NotFound, "error.ai.memoryNotFound");
    }
    err(ErrorCode::Internal, "error.file.write").arg("reason", e.to_string())
}

/// Every memory file that exists, as relative paths.
fn files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for sub in ["", "people", "sites", "projects"] {
        let Ok(entries) = std::fs::read_dir(dir.join(sub)) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = if sub.is_empty() {
                name
            } else {
                format!("{sub}/{name}")
            };
            if memory::valid_path(&rel) && e.path().is_file() {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

fn mtime(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
}

/// Brings the index up to date with the files (13 §10): a file is indexed
/// again when its modification time changed.
fn reindex(c: &rusqlite::Connection, dir: &Path) -> rusqlite::Result<()> {
    let now = files(dir);
    let known: BTreeMap<String, i64> = c
        .prepare("SELECT path, mtime FROM ai_memory_files")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for gone in known.keys().filter(|p| !now.contains(p)) {
        c.execute("DELETE FROM ai_memory_fts WHERE path = ?1", [gone])?;
        c.execute("DELETE FROM ai_memory_files WHERE path = ?1", [gone])?;
    }
    for path in &now {
        let m = mtime(&dir.join(path));
        if known.get(path) == Some(&m) {
            continue;
        }
        let text = std::fs::read_to_string(dir.join(path)).unwrap_or_default();
        c.execute("DELETE FROM ai_memory_fts WHERE path = ?1", [path])?;
        for p in memory::paragraphs(&text) {
            c.execute(
                "INSERT INTO ai_memory_fts (path, heading, body, line) VALUES (?1, ?2, ?3, ?4)",
                params![path, p.heading, p.body, i64::try_from(p.line).unwrap_or(0)],
            )?;
        }
        c.execute(
            "INSERT INTO ai_memory_files (path, mtime) VALUES (?1, ?2) ON CONFLICT (path) DO UPDATE SET mtime = excluded.mtime",
            params![path, m],
        )?;
    }
    Ok(())
}

fn search(c: &rusqlite::Connection, dir: &Path, query: &str) -> rusqlite::Result<Vec<Value>> {
    reindex(c, dir)?;
    let q = memory::fts_query(query);
    if q.is_empty() {
        return Ok(Vec::new());
    }
    c.prepare("SELECT path, line, heading, body FROM ai_memory_fts WHERE ai_memory_fts MATCH ?1 ORDER BY rank LIMIT 20")?
        .query_map([q], |r| {
            Ok(json!({ "path": r.get::<_, String>(0)?, "line": r.get::<_, i64>(1)?, "heading": r.get::<_, String>(2)?, "text": r.get::<_, String>(3)? }))
        })?
        .collect()
}

fn send(reply: Reply, r: Result<Value, MadoError>) {
    let _ = reply.send(r);
}

/// Writes `text` to `path`, making its folder.
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, text)
}

/// One day's filing: files as they were before (`null`: did not exist)
/// and the inbox it filed.
fn history(dir: &Path, day: &str) -> PathBuf {
    dir.join(".history").join(format!("{day}.json"))
}

impl Core {
    /// `<profile>/AI/`.
    pub(super) fn ai_dir(&self) -> PathBuf {
        self.backups
            .parent()
            .map_or_else(std::env::temp_dir, Path::to_path_buf)
            .join("AI")
    }

    fn memory_dir(&self) -> PathBuf {
        self.ai_dir().join("memory")
    }

    pub(super) fn memory_on(&self) -> bool {
        self.setting("features.ai") == true
            && self.setting("features.memory") == true
            && !self.state.profile.is_private
    }

    /// The skills, the person's first (13 §11). Reads the profile's folder.
    pub(super) fn skills(&self) -> Vec<Skill> {
        skills::all(&self.ai_dir().join("skills"))
    }

    /// The task prompt's skills index.
    pub(super) fn skills_index(&self) -> String {
        skills::index(&self.skills())
    }

    /// `ai.memory.*` and `ai.skill.*`.
    pub(super) fn memory_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        reply_to: &mut Option<Reply>,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let protected = matches!(
            cmd,
            Command::AiMemoryWrite { .. }
                | Command::AiMemoryDelete { .. }
                | Command::AiMemoryFile
                | Command::AiMemoryUndo { .. }
                | Command::AiSkillDelete { .. }
        );
        if protected && !origin.is_user() {
            return Err(err(ErrorCode::Protected, "error.ai.notAllowed"));
        }
        let dir = self.memory_dir();
        match cmd {
            Command::AiSkillList => Ok(json!(self.skills())),
            Command::AiSkillRead { name } => {
                let s = self
                    .skills()
                    .into_iter()
                    .find(|s| s.name == name)
                    .ok_or_else(|| err(ErrorCode::NotFound, "error.ai.noSkill"))?;
                Ok(json!({ "name": s.name, "document": s.document, "builtin": s.builtin }))
            }
            Command::AiSkillDelete { name } => {
                if !skills::valid_name(&name) {
                    return Err(err(ErrorCode::NotFound, "error.ai.noSkill"));
                }
                let folder = self.ai_dir().join("skills").join(&name);
                std::fs::remove_dir_all(&folder).map_err(|e| io(&e))?;
                Ok(Value::Null)
            }
            _ if !self.memory_on() => Err(off()),
            Command::AiMemoryList => Ok(Value::Array(
                files(&dir)
                    .into_iter()
                    .map(|p| {
                        let m = std::fs::metadata(dir.join(&p)).ok();
                        json!({ "path": p, "bytes": m.as_ref().map_or(0, std::fs::Metadata::len), "modifiedAt": mtime(&dir.join(&p)) / 1_000_000 })
                    })
                    .collect(),
            )),
            Command::AiMemoryRead { path } => {
                if !memory::valid_path(&path) {
                    return Err(bad_path());
                }
                let text = std::fs::read_to_string(dir.join(&path)).map_err(|e| io(&e))?;
                Ok(json!({ "path": path, "text": text }))
            }
            Command::AiMemoryWrite { path, text } => {
                if !memory::valid_path(&path) {
                    return Err(bad_path());
                }
                write(&dir.join(&path), &text).map_err(|e| io(&e))?;
                Ok(Value::Null)
            }
            Command::AiMemoryDelete { path } => {
                if !memory::valid_path(&path) {
                    return Err(bad_path());
                }
                std::fs::remove_file(dir.join(&path)).map_err(|e| io(&e))?;
                Ok(Value::Null)
            }
            Command::AiMemorySearch { query } => {
                let Some(reply) = reply_to.take() else {
                    return Ok(Value::Null);
                };
                let pending = self
                    .profile_db
                    .writer()
                    .write(move |c| search(c, &dir, &query));
                tokio::task::spawn_local(async move {
                    let _ = reply.send(pending.await.map(Value::Array).map_err(|_| MadoError::internal()));
                });
                Ok(Value::Null)
            }
            Command::AiMemoryFile => {
                self.file_memory(reply_to.take());
                Ok(Value::Null)
            }
            Command::AiMemoryUndo { day } => {
                if day.len() != 10 || !day.chars().all(|c| c.is_ascii_digit() || c == '-') {
                    return Err(err(ErrorCode::InvalidInput, "error.ai.memoryDay"));
                }
                let h = history(&dir, &day);
                let saved: Value = std::fs::read_to_string(&h)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .ok_or_else(|| err(ErrorCode::NotFound, "error.ai.memoryNoFiling"))?;
                for (path, before) in saved["files"].as_object().into_iter().flatten() {
                    if !memory::valid_path(path) {
                        continue;
                    }
                    let r = match before.as_str() {
                        Some(t) => write(&dir.join(path), t),
                        None => std::fs::remove_file(dir.join(path)),
                    };
                    r.map_err(|e| io(&e))?;
                }
                // The filed entries go back to the inbox, before newer ones.
                let newer = std::fs::read_to_string(dir.join(INBOX)).unwrap_or_default();
                let filed = saved["inbox"].as_str().unwrap_or_default();
                write(&dir.join(INBOX), &format!("{filed}{newer}")).map_err(|e| io(&e))?;
                let _ = std::fs::remove_file(&h);
                let _ = rec;
                Ok(Value::Null)
            }
            _ => Err(err(ErrorCode::Unsupported, "error.unknownCommand")),
        }
    }

    /// The memory and skill tools (13 §8, §10, §11).
    pub(super) fn memory_tool(
        &mut self,
        origin: Origin,
        name: &str,
        args: Value,
        reply: Reply,
        rec: &mut PatchRecorder,
    ) {
        let dir = self.memory_dir();
        match name {
            "skills" => send(reply, Ok(json!(self.skills()))),
            "skill" => {
                let wanted = args["name"].as_str().unwrap_or_default();
                send(
                    reply,
                    self.skills()
                        .into_iter()
                        .find(|s| s.name == wanted)
                        .map(|s| json!({ "name": s.name, "document": s.document }))
                        .ok_or_else(|| err(ErrorCode::NotFound, "error.ai.noSkill")),
                );
            }
            "skill_save" => self.save_skill(origin, &args, reply, rec),
            _ if !self.memory_on() => send(reply, Err(off())),
            "memory_read" => {
                let path = args["path"].as_str().unwrap_or_default();
                send(
                    reply,
                    if memory::valid_path(path) {
                        std::fs::read_to_string(dir.join(path))
                            .map(|t| json!({ "path": path, "text": t }))
                            .map_err(|e| io(&e))
                    } else {
                        Err(bad_path())
                    },
                );
            }
            "memory_search" => {
                let query = args["query"].as_str().unwrap_or_default().to_owned();
                let pending = self
                    .profile_db
                    .writer()
                    .write(move |c| search(c, &dir, &query));
                tokio::task::spawn_local(async move {
                    let r = pending
                        .await
                        .map(|v| json!({ "results": v }))
                        .map_err(|_| MadoError::internal());
                    let _ = reply.send(r);
                });
            }
            "remember" => {
                let text = args["text"].as_str().unwrap_or_default().trim().to_owned();
                if text.is_empty() {
                    send(
                        reply,
                        Err(err(ErrorCode::InvalidInput, "error.ai.memoryEmpty")),
                    );
                    return;
                }
                let task = self
                    .task_of(&origin, rec)
                    .map_or_else(|| "person".to_owned(), |t| t.to_string());
                let (_, time) = memory::utc(self.now());
                // The vault's passwords, when it is open (13 §10).
                let vault = self.vault.clone().zip(self.vault_keys.clone());
                tokio::task::spawn_blocking(move || {
                    let passwords = vault
                        .map(|(cfg, keys)| super::health::vault_passwords(&cfg, &keys))
                        .unwrap_or_default();
                    let r = match memory::refuse(&text, &passwords) {
                        Some(why) => Err(err(ErrorCode::InvalidInput, why.message_key())),
                        None => {
                            use std::io::Write as _;
                            std::fs::create_dir_all(&dir)
                                .and_then(|()| {
                                    std::fs::OpenOptions::new()
                                        .create(true)
                                        .append(true)
                                        .open(dir.join(INBOX))
                                })
                                .and_then(|mut f| {
                                    f.write_all(memory::inbox_entry(&time, &task, &text).as_bytes())
                                })
                                .map(|()| json!({ "remembered": true }))
                                .map_err(|e| io(&e))
                        }
                    };
                    let _ = reply.send(r);
                });
            }
            _ => send(
                reply,
                Err(err(ErrorCode::NotFound, "error.control.unknownMethod")),
            ),
        }
    }

    /// `skill_save` (13 §11): the document in an approval card; written
    /// only on Allow.
    fn save_skill(&mut self, origin: Origin, args: &Value, reply: Reply, rec: &mut PatchRecorder) {
        let name = args["name"].as_str().unwrap_or_default().to_owned();
        let document = args["document"].as_str().unwrap_or_default().to_owned();
        let skill = match skills::parse(&document) {
            Ok(s) if s.name == name => s,
            Ok(_) => {
                let _ = reply.send(Err(err(ErrorCode::InvalidInput, "error.ai.skillName")));
                return;
            }
            Err(why) => {
                let _ =
                    reply
                        .send(Err(err(ErrorCode::InvalidInput, "error.ai.skillInvalid")
                            .arg("reason", why)));
                return;
            }
        };
        let folder = self.ai_dir().join("skills").join(&skill.name);
        if folder.join("SKILL.md").exists() && args["replacing"] != true {
            let _ = reply.send(Err(err(ErrorCode::Conflict, "error.ai.skillExists")));
            return;
        }
        let (tx, rx) = oneshot::channel();
        self.ask_approval(
            super::safety::Ask {
                origin,
                tab: None,
                class: mado_agent::safety::Class::Skill,
                url: String::new(),
                detail: format!("Save the skill \"{}\":\n\n{document}", skill.name),
                amount: None,
            },
            tx,
            rec,
        );
        tokio::task::spawn_local(async move {
            let r = match rx.await {
                Ok(Ok(())) => tokio::task::spawn_blocking(move || {
                    write(&folder.join("SKILL.md"), &document)
                        .map(|()| json!({ "saved": skill.name }))
                        .map_err(|e| io(&e))
                })
                .await
                .unwrap_or_else(|_| Err(MadoError::internal())),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(MadoError::internal()),
            };
            let _ = reply.send(r);
        });
    }

    /// The hint an agent gets the first time one of its tabs lands on a
    /// skill's site (13 §11).
    pub(super) fn skill_hint(
        &mut self,
        origin: &Origin,
        url: &str,
        rec: &mut PatchRecorder,
    ) -> Option<String> {
        let task = self.task_of(origin, rec)?;
        let host = url::Url::parse(url).ok()?.host_str()?.to_owned();
        let skills = self.skills();
        let s = skills::for_host(&skills, &host)?;
        let seen = self.skill_hints.entry(task).or_default();
        seen.insert(s.name.clone())
            .then(|| format!("Skill available: {}", s.name))
    }

    /// A task ended: its hints go.
    pub(super) fn forget_skill_hints(&mut self, task: TaskId) {
        self.skill_hints.remove(&task);
    }

    /// The daily filing (13 §10): the `fast` model moves the inbox into the
    /// other files; the files as they were are kept for `ai.memory.undo`.
    pub(super) fn file_memory(&mut self, reply: Option<Reply>) {
        let done = |reply: Option<Reply>, r: Result<Value, MadoError>| {
            if let Some(reply) = reply {
                let _ = reply.send(r);
            }
        };
        let dir = self.memory_dir();
        let inbox = std::fs::read_to_string(dir.join(INBOX)).unwrap_or_default();
        let (day, _) = memory::utc(self.now());
        if !self.memory_on() || inbox.trim().is_empty() {
            done(reply, Ok(json!({ "day": day, "files": [] })));
            return;
        }
        let attempts = self.ai_attempts("fast");
        let Some(at) = attempts.first().cloned() else {
            done(reply, Err(err(ErrorCode::Unsupported, "error.ai.noModel")));
            return;
        };
        let current: BTreeMap<String, String> = files(&dir)
            .into_iter()
            .filter(|p| p != INBOX)
            .filter_map(|p| std::fs::read_to_string(dir.join(&p)).ok().map(|t| (p, t)))
            .collect();
        let req = memory::filing_request(&at.model, &inbox, &current);
        let secrets = std::sync::Arc::clone(&self.ai.secrets);
        tokio::task::spawn_local(async move {
            let r = async {
                let turn = super::chat::answer(&secrets, &at, &req, |_| {})
                    .await
                    .map_err(|e| {
                        err(
                            ErrorCode::Internal,
                            &format!("error.ai.{}", e.failure().key()),
                        )
                    })?;
                let changed = memory::parse_filing(&turn.text).map_err(|why| {
                    err(ErrorCode::Internal, "error.ai.memoryFiling").arg("reason", why)
                })?;
                tokio::task::spawn_blocking(move || -> Result<Value, MadoError> {
                    let before: BTreeMap<&String, Option<&String>> =
                        changed.keys().map(|p| (p, current.get(p))).collect();
                    // Entries added while the model worked stay in the inbox.
                    let now = std::fs::read_to_string(dir.join(INBOX)).unwrap_or_default();
                    let h = history(&dir, &day);
                    let mut saved: Value = std::fs::read_to_string(&h)
                        .ok()
                        .and_then(|t| serde_json::from_str(&t).ok())
                        .unwrap_or_else(|| json!({ "files": {}, "inbox": "" }));
                    for (p, b) in &before {
                        if saved["files"].get(p.as_str()).is_none() {
                            saved["files"][p.as_str()] = json!(b);
                        }
                    }
                    saved["inbox"] = json!(format!(
                        "{}{inbox}",
                        saved["inbox"].as_str().unwrap_or_default()
                    ));
                    write(&h, &saved.to_string()).map_err(|e| io(&e))?;
                    for (p, text) in &changed {
                        write(&dir.join(p), text).map_err(|e| io(&e))?;
                    }
                    write(
                        &dir.join(INBOX),
                        now.strip_prefix(inbox.as_str()).unwrap_or_default(),
                    )
                    .map_err(|e| io(&e))?;
                    Ok(json!({ "day": day, "files": changed.keys().collect::<Vec<_>>() }))
                })
                .await
                .unwrap_or_else(|_| Err(MadoError::internal()))
            }
            .await;
            if let Err(e) = &r {
                tracing::warn!(error = %e.message_key, "the memory inbox was not filed");
            }
            done(reply, r);
        });
    }
}

/// Skill hints already given, by task.
pub(super) type SkillHints = BTreeMap<TaskId, BTreeSet<String>>;
