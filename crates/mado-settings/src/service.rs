//! Values, protection, the change log and undo, search, export and import
//! (spec 06 §2, §3, §5). The service is plain logic over an in-memory copy
//! of the stored values: each change returns the rows to write, which the
//! caller queues on the storage writer (`persist`).

use std::collections::{BTreeMap, VecDeque};

use mado_types::command::{Origin, ResetTarget};
use mado_types::{ErrorCode, MadoError};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::setting::{SettingDef, lookup};
use crate::strings::Strings;
use crate::types::{Scope, Section};
use crate::{REGISTRY, Setting, def};

/// Entries kept in `settings_log` (spec 03 §7).
const LOG_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SettingsError {
    #[error("no setting {0}")]
    UnknownKey(String),
    #[error("{key}: {reason}")]
    Invalid { key: String, reason: String },
    #[error("{0} can be changed only in the settings window")]
    Protected(String),
    #[error("{0} needs Touch ID or your password")]
    AuthRequired(String),
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("no section {0}")]
    UnknownSection(String),
    #[error("file: {0}")]
    File(String),
}

impl From<SettingsError> for MadoError {
    fn from(e: SettingsError) -> Self {
        match e {
            SettingsError::UnknownKey(key) => {
                MadoError::new(ErrorCode::NotFound, "error.settings.unknownKey").arg("key", key)
            }
            SettingsError::Invalid { key, reason } => {
                MadoError::new(ErrorCode::InvalidInput, "error.settings.invalidValue")
                    .arg("key", key)
                    .arg("reason", reason)
            }
            SettingsError::Protected(key) => {
                MadoError::new(ErrorCode::Protected, "error.settings.protected").arg("key", key)
            }
            SettingsError::AuthRequired(key) => {
                MadoError::new(ErrorCode::PermissionDenied, "error.settings.authRequired")
                    .arg("key", key)
            }
            SettingsError::NothingToUndo => {
                MadoError::new(ErrorCode::NotFound, "error.settings.nothingToUndo")
            }
            SettingsError::UnknownSection(section) => {
                MadoError::new(ErrorCode::NotFound, "error.settings.unknownSection")
                    .arg("section", section)
            }
            SettingsError::File(reason) => {
                MadoError::new(ErrorCode::InvalidInput, "error.settings.file").arg("reason", reason)
            }
        }
    }
}

/// One row of `settings_log`.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub id: i64,
    pub key: String,
    /// `None`: the default.
    pub old: Option<Value>,
    pub new: Option<Value>,
    pub origin: String,
    pub at: i64,
}

/// A change in effect: what the core patches into `state.settings` and
/// hands to the setting's side effect (spec 06 §4).
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub key: &'static str,
    pub old: Value,
    pub new: Value,
}

/// A row to write. App-scope values go to `app.sqlite`, everything else to
/// `profile.sqlite`.
#[derive(Debug, Clone, PartialEq)]
pub enum Write {
    Upsert {
        scope: Scope,
        key: &'static str,
        value: Value,
        version: u32,
        at: i64,
    },
    Delete {
        scope: Scope,
        key: &'static str,
    },
    Log(LogEntry),
    DeleteLog {
        id: i64,
    },
}

/// What a command did. `undo_notice` is the log entry to offer Undo for.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outcome {
    pub changes: Vec<Change>,
    pub writes: Vec<Write>,
    pub undo_notice: Option<i64>,
}

/// Checks the one-time token from `app.authenticate` (spec 06 §5).
pub trait AuthTokens {
    /// True if `token` is valid; a token works once.
    fn consume(&mut self, token: &str) -> bool;
}

/// The import report (spec 06 §3).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub applied: Vec<String>,
    pub unchanged: Vec<String>,
    pub skipped_unknown: Vec<String>,
    pub skipped_protected: Vec<String>,
    pub invalid: Vec<String>,
}

/// A search hit (spec 06 §3).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub key: &'static str,
    pub section: &'static str,
    pub score: u32,
}

#[derive(Debug, Clone)]
pub struct Settings {
    /// Stored (non-default) values.
    values: BTreeMap<&'static str, Value>,
    log: VecDeque<LogEntry>,
    next_log_id: i64,
    strings: Strings,
}

fn requires_auth(def: &SettingDef, new: &Value) -> bool {
    def.key.starts_with("vault.")
        || def.key == "developer.controlEnabled"
        || (def.key == "ai.approvalMode" && new == "full")
}

fn may_change(def: &SettingDef, origin: &Origin) -> bool {
    !def.opts.protected || matches!(origin, Origin::Settings | Origin::System)
}

fn origin_name(origin: &Origin) -> String {
    match origin {
        Origin::Ui { .. } => "ui".into(),
        Origin::Overlay { .. } => "overlay".into(),
        Origin::Settings => "settings".into(),
        Origin::Control { client } => format!("control:{client}"),
        Origin::Agent { task } => format!("agent:{task}"),
        Origin::System => "system".into(),
    }
}

impl Settings {
    /// An empty store: every setting at its default.
    pub fn new(strings: Strings) -> Self {
        Self {
            values: BTreeMap::new(),
            log: VecDeque::new(),
            next_log_id: 1,
            strings,
        }
    }

    /// Loads stored values from both databases. Values from an older
    /// `version` go through the setting's `migrate`; invalid or
    /// unmigratable ones fall back to the default, and the writes returned
    /// remove them and log it (spec 06 §2).
    pub fn load(
        app: &Connection,
        profile: &Connection,
        strings: Strings,
        now: i64,
    ) -> rusqlite::Result<(Self, Vec<Write>)> {
        let mut me = Self::new(strings);
        let mut fixes = Vec::new();
        let mut stmt = profile
            .prepare("SELECT id, key, old, new, origin, at FROM settings_log ORDER BY id")?;
        let entries: Vec<LogEntry> = stmt
            .query_map([], |r| {
                let old: Option<String> = r.get(2)?;
                let new: Option<String> = r.get(3)?;
                Ok(LogEntry {
                    id: r.get(0)?,
                    key: r.get(1)?,
                    old: old.and_then(|s| serde_json::from_str(&s).ok()),
                    new: new.and_then(|s| serde_json::from_str(&s).ok()),
                    origin: r.get(4)?,
                    at: r.get(5)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        me.next_log_id = me.next_log_id.max(entries.last().map_or(1, |e| e.id + 1));
        me.log.extend(entries);
        for (conn, scope) in [(app, Scope::App), (profile, Scope::Profile)] {
            let mut stmt = conn.prepare("SELECT key, value, schema_version FROM settings")?;
            let rows: Vec<(String, String, i64)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            for (key, text, stored_version) in rows {
                let Some(def) = lookup(&key).filter(|d| d.scope == scope) else {
                    // A key this build does not know: leave it for a newer one.
                    continue;
                };
                let parsed: Option<Value> = serde_json::from_str(&text).ok();
                let version = u32::try_from(stored_version).unwrap_or(0);
                let value = parsed.and_then(|v| {
                    if version < def.opts.version {
                        def.opts.migrate.and_then(|m| m(version, &v))
                    } else {
                        Some(v)
                    }
                });
                match value.filter(|v| def.ty.validate(v).is_ok()) {
                    Some(v) => {
                        if version < def.opts.version {
                            fixes.push(Write::Upsert {
                                scope,
                                key: def.key,
                                value: v.clone(),
                                version: def.opts.version,
                                at: now,
                            });
                        }
                        me.values.insert(def.key, v);
                    }
                    None => {
                        fixes.push(Write::Delete {
                            scope,
                            key: def.key,
                        });
                        let entry = me.log_entry(
                            def.key,
                            serde_json::from_str(&text).ok(),
                            None,
                            "migration".into(),
                            now,
                        );
                        fixes.push(Write::Log(entry));
                    }
                }
            }
        }
        Ok((me, fixes))
    }

    pub fn strings(&self) -> &Strings {
        &self.strings
    }

    /// The value in effect.
    pub fn value(&self, def: &SettingDef) -> Value {
        self.values
            .get(def.key)
            .cloned()
            .unwrap_or_else(|| def.default_value())
    }

    /// The typed value of a setting.
    pub fn get<S: Setting>(&self) -> S::Value {
        let def = def(S::KEY);
        serde_json::from_value(self.value(def)).unwrap_or_else(|_| {
            serde_json::from_value(def.default_value())
                .expect("registry defaults match their Rust type")
        })
    }

    /// `settings.get`: `{ key: value }` for `keys`, or every setting.
    pub fn get_many(&self, keys: Option<&[String]>) -> Result<Value, SettingsError> {
        let mut out = serde_json::Map::new();
        match keys {
            None => {
                for def in REGISTRY {
                    out.insert(def.key.into(), self.value(def));
                }
            }
            Some(keys) => {
                for key in keys {
                    let def = lookup(key).ok_or_else(|| SettingsError::UnknownKey(key.clone()))?;
                    out.insert(def.key.into(), self.value(def));
                }
            }
        }
        Ok(Value::Object(out))
    }

    /// Keys whose value differs from the default.
    pub fn changed(&self) -> Vec<&'static str> {
        self.values.keys().copied().collect()
    }

    /// Values mirrored into `state.settings`: settings with `ui: true`.
    pub fn ui_values(&self) -> BTreeMap<String, Value> {
        REGISTRY
            .iter()
            .filter(|d| d.opts.ui)
            .map(|d| (d.key.to_owned(), self.value(d)))
            .collect()
    }

    fn log_entry(
        &mut self,
        key: &str,
        old: Option<Value>,
        new: Option<Value>,
        origin: String,
        at: i64,
    ) -> LogEntry {
        let entry = LogEntry {
            id: self.next_log_id,
            key: key.to_owned(),
            old,
            new,
            origin,
            at,
        };
        self.next_log_id += 1;
        self.log.push_back(entry.clone());
        entry
    }

    /// Changes one value with no checks beyond validation; every public
    /// path checks protection first. `None` means the default.
    fn apply(
        &mut self,
        def: &'static SettingDef,
        new: Option<Value>,
        origin: &Origin,
        at: i64,
        out: &mut Outcome,
    ) {
        let old_stored = self.values.get(def.key).cloned();
        let default = def.default_value();
        // Storing the default is the same as storing nothing.
        let new = new.filter(|v| *v != default);
        if old_stored == new {
            return;
        }
        let old_value = old_stored.clone().unwrap_or_else(|| default.clone());
        let new_value = new.clone().unwrap_or(default);
        match &new {
            Some(v) => {
                self.values.insert(def.key, v.clone());
                out.writes.push(Write::Upsert {
                    scope: def.scope,
                    key: def.key,
                    value: v.clone(),
                    version: def.opts.version,
                    at,
                });
            }
            None => {
                self.values.remove(def.key);
                out.writes.push(Write::Delete {
                    scope: def.scope,
                    key: def.key,
                });
            }
        }
        let entry = self.log_entry(def.key, old_stored, new, origin_name(origin), at);
        if def.opts.undo_notice {
            out.undo_notice = Some(entry.id);
        }
        out.writes.push(Write::Log(entry));
        while self.log.len() > LOG_LIMIT {
            if let Some(dropped) = self.log.pop_front() {
                out.writes.push(Write::DeleteLog { id: dropped.id });
            }
        }
        out.changes.push(Change {
            key: def.key,
            old: old_value,
            new: new_value,
        });
    }

    fn checked(&self, key: &str, origin: &Origin) -> Result<&'static SettingDef, SettingsError> {
        let def = lookup(key).ok_or_else(|| SettingsError::UnknownKey(key.to_owned()))?;
        if def.scope == Scope::Site {
            return Err(SettingsError::UnknownKey(key.to_owned()));
        }
        if !may_change(def, origin) {
            return Err(SettingsError::Protected(key.to_owned()));
        }
        Ok(def)
    }

    /// `settings.set`. Fails without changing anything when the key is
    /// unknown, the value invalid, the key protected for `origin`, or OS
    /// authentication is needed and `token` is not valid.
    pub fn set(
        &mut self,
        key: &str,
        value: Value,
        origin: &Origin,
        token: Option<&str>,
        auth: &mut dyn AuthTokens,
        at: i64,
    ) -> Result<Outcome, SettingsError> {
        let def = self.checked(key, origin)?;
        def.ty
            .validate(&value)
            .map_err(|reason| SettingsError::Invalid {
                key: key.to_owned(),
                reason,
            })?;
        if value != self.value(def)
            && requires_auth(def, &value)
            && !token.is_some_and(|t| auth.consume(t))
        {
            return Err(SettingsError::AuthRequired(key.to_owned()));
        }
        let mut out = Outcome::default();
        self.apply(def, Some(value), origin, at, &mut out);
        Ok(out)
    }

    /// `settings.reset`. A section or `all` resets every key the origin may
    /// change and skips protected ones; a single protected key fails.
    pub fn reset(
        &mut self,
        target: &ResetTarget,
        origin: &Origin,
        at: i64,
    ) -> Result<Outcome, SettingsError> {
        let defs: Vec<&'static SettingDef> = match target {
            ResetTarget::Key { key } => vec![self.checked(key, origin)?],
            ResetTarget::Section { section } => {
                let section = Section::from_id(section)
                    .ok_or_else(|| SettingsError::UnknownSection(section.clone()))?;
                REGISTRY
                    .iter()
                    .filter(|d| {
                        d.section == section && d.scope != Scope::Site && may_change(d, origin)
                    })
                    .collect()
            }
            ResetTarget::All { all: true } => REGISTRY
                .iter()
                .filter(|d| d.scope != Scope::Site && may_change(d, origin))
                .collect(),
            ResetTarget::All { all: false } => vec![],
        };
        let mut out = Outcome::default();
        for def in defs {
            self.apply(def, None, origin, at, &mut out);
        }
        out.undo_notice = None;
        Ok(out)
    }

    /// `settings.undo`: puts back the value before the newest change, or
    /// before change `log_id`, and removes that entry from the log.
    pub fn undo(
        &mut self,
        log_id: Option<i64>,
        origin: &Origin,
        at: i64,
    ) -> Result<Outcome, SettingsError> {
        let index = match log_id {
            Some(id) => self.log.iter().position(|e| e.id == id),
            None => self.log.len().checked_sub(1),
        }
        .ok_or(SettingsError::NothingToUndo)?;
        let entry = self.log[index].clone();
        let def = self.checked(&entry.key, origin)?;
        let mut out = Outcome::default();
        self.apply(def, entry.old.clone(), origin, at, &mut out);
        // The undo itself is not a change to undo again.
        let removed: Vec<i64> = out
            .writes
            .iter()
            .filter_map(|w| match w {
                Write::Log(e) => Some(e.id),
                _ => None,
            })
            .chain([entry.id])
            .collect();
        self.log.retain(|e| !removed.contains(&e.id));
        out.writes.retain(|w| !matches!(w, Write::Log(_)));
        out.writes
            .extend(removed.into_iter().map(|id| Write::DeleteLog { id }));
        out.undo_notice = None;
        Ok(out)
    }

    pub fn log(&self) -> impl Iterator<Item = &LogEntry> {
        self.log.iter()
    }

    fn feature_on(&self, feature: &str) -> bool {
        lookup(&format!("features.{feature}")).is_some_and(|d| self.value(d) == true)
    }

    /// `settings.search`: ranked by label, then keywords, help and the
    /// current value's text. Settings of a feature that is off are hidden.
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<SearchHit> = REGISTRY
            .iter()
            .filter(|d| d.opts.feature.is_none_or(|f| self.feature_on(f)))
            .filter_map(|d| {
                let fields = [
                    (self.strings.get(d.label).to_lowercase(), 8),
                    (self.strings.get(d.keywords).to_lowercase(), 4),
                    (self.strings.get(d.help).to_lowercase(), 2),
                    (self.value(d).to_string().to_lowercase(), 1),
                ];
                // Per word: the best field's weight, plus one for every other
                // field that also mentions it.
                let mut score = 0;
                for word in &words {
                    let matching: Vec<u32> = fields
                        .iter()
                        .filter(|(text, _)| text.contains(word.as_str()))
                        .map(|(_, w)| *w)
                        .collect();
                    let extra = u32::try_from(matching.len()).unwrap_or(1).saturating_sub(1);
                    score += matching.iter().max()? + extra;
                }
                Some(SearchHit {
                    key: d.key,
                    section: d.section.id(),
                    score,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.key.cmp(b.key)));
        hits
    }

    /// `settings.export`: changed values, without protected ones.
    pub fn export(&self) -> Value {
        let mut app = serde_json::Map::new();
        let mut profile = serde_json::Map::new();
        for (key, value) in &self.values {
            let Some(def) = lookup(key).filter(|d| !d.opts.protected) else {
                continue;
            };
            match def.scope {
                Scope::App => app.insert((*key).into(), value.clone()),
                _ => profile.insert((*key).into(), value.clone()),
            };
        }
        json!({ "madoSettings": 1, "app": app, "profile": profile })
    }

    /// `settings.import`: validates every key, skips unknown and protected
    /// ones, applies the rest.
    pub fn import(
        &mut self,
        file: &Value,
        origin: &Origin,
        at: i64,
    ) -> Result<(Outcome, ImportReport), SettingsError> {
        if file.get("madoSettings") != Some(&json!(1)) {
            return Err(SettingsError::File("not a Mado settings file".into()));
        }
        let mut out = Outcome::default();
        let mut report = ImportReport::default();
        for part in ["app", "profile"] {
            let Some(values) = file.get(part).and_then(Value::as_object) else {
                continue;
            };
            for (key, value) in values {
                let Some(def) = lookup(key).filter(|d| d.scope != Scope::Site) else {
                    report.skipped_unknown.push(key.clone());
                    continue;
                };
                if def.opts.protected {
                    report.skipped_protected.push(key.clone());
                    continue;
                }
                if def.ty.validate(value).is_err() {
                    report.invalid.push(key.clone());
                    continue;
                }
                let before = out.changes.len();
                self.apply(def, Some(value.clone()), origin, at, &mut out);
                if out.changes.len() > before {
                    report.applied.push(key.clone());
                } else {
                    report.unchanged.push(key.clone());
                }
            }
        }
        out.undo_notice = None;
        Ok((out, report))
    }
}

/// Writes the rows that belong to `database` (`Scope::App` for
/// `app.sqlite`, `Scope::Profile` for `profile.sqlite`, which also holds the
/// log). Run it once per database, inside that database's writer.
pub fn persist(writes: &[Write], database: Scope, conn: &Connection) -> rusqlite::Result<()> {
    let ours = |scope: &Scope| (*scope == Scope::App) == (database == Scope::App);
    for write in writes {
        match write {
            Write::Upsert {
                scope,
                key,
                value,
                version,
                at,
            } if ours(scope) => {
                conn.execute(
                    "INSERT INTO settings (key, value, schema_version, updated_at) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value, schema_version = excluded.schema_version, updated_at = excluded.updated_at",
                    params![key, value.to_string(), version, at],
                )?;
            }
            Write::Delete { scope, key } if ours(scope) => {
                conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
            }
            Write::Log(e) if database == Scope::Profile => {
                conn.execute(
                    "INSERT INTO settings_log (id, key, old, new, origin, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![e.id, e.key, e.old.as_ref().map(Value::to_string), e.new.as_ref().map(Value::to_string), e.origin, e.at],
                )?;
            }
            Write::DeleteLog { id } if database == Scope::Profile => {
                conn.execute("DELETE FROM settings_log WHERE id = ?1", [id])?;
            }
            _ => {}
        }
    }
    Ok(())
}
