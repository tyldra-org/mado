//! Overlaps and takeovers (spec 09 §8): an extension that does what a Mado
//! feature does asks which should run; one that replaces the new tab page,
//! the search engine or the startup pages asks whether that was expected.
//! What either changed goes back when the extension is turned off or
//! removed.

use std::collections::BTreeMap;

use mado_types::command::{Command, Origin, OverlapChoice};
use mado_types::patch::PatchRecorder;
use mado_types::state::{Notice, PromptState};
use mado_types::{ErrorCode, ExtensionId, MadoError, PromptId};
use rusqlite::params;
use serde_json::{Value, json};

use super::Core;

/// A category's stored choice.
#[derive(Debug, Clone)]
pub struct OverlapRow {
    pub choice: OverlapChoice,
    pub extension_id: Option<ExtensionId>,
    /// Setting key to its value before the choice.
    pub previous: BTreeMap<String, Value>,
}

/// A takeover the user saw.
#[derive(Debug, Clone)]
pub struct TakeoverRow {
    pub kept: bool,
    pub previous: BTreeMap<String, Value>,
}

/// Takeovers by extension and what it took over.
pub type Takeovers = BTreeMap<(ExtensionId, String), TakeoverRow>;

/// Overlap choices and takeovers, from the profile database.
pub(super) fn load(
    conn: &rusqlite::Connection,
) -> rusqlite::Result<(BTreeMap<String, OverlapRow>, Takeovers)> {
    let choice = |s: &str| match s {
        "both" => OverlapChoice::Both,
        "madoOnly" => OverlapChoice::MadoOnly,
        "madoConnect" => OverlapChoice::MadoConnect,
        _ => OverlapChoice::ExtensionOnly,
    };
    let map = |s: String| serde_json::from_str(&s).unwrap_or_default();
    let overlaps = conn
        .prepare("SELECT category, choice, extension_id, previous FROM overlaps")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                OverlapRow {
                    choice: choice(&r.get::<_, String>(1)?),
                    extension_id: r.get::<_, Option<String>>(2)?.and_then(|s| s.parse().ok()),
                    previous: map(r.get(3)?),
                },
            ))
        })?
        .filter_map(Result::ok)
        .collect();
    let takeovers = conn
        .prepare("SELECT extension_id, what, kept, previous FROM extension_takeovers")?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                TakeoverRow {
                    kept: r.get(2)?,
                    previous: map(r.get(3)?),
                },
            ))
        })?
        .filter_map(Result::ok)
        .filter_map(|(id, what, row)| Some(((id.parse().ok()?, what), row)))
        .collect();
    Ok((overlaps, takeovers))
}

/// The overlap category of an extension (09 §8): the compatibility list's,
/// or `adBlocker` for more than 5,000 declarative net request rules.
fn category_of(id: &ExtensionId, dnr_rules: usize) -> Option<&'static str> {
    use mado_lists::Category as C;
    match mado_lists::compat()
        .extensions
        .get(id.as_str())
        .map(|e| e.category)
    {
        Some(C::AdBlocker) => Some("adBlocker"),
        Some(C::PasswordManager) => Some("passwordManager"),
        Some(C::DarkMode) => Some("darkMode"),
        Some(C::Translation) => Some("translation"),
        Some(C::TabSleeping) => Some("tabSleeping"),
        _ if dnr_rules > 5_000 => Some("adBlocker"),
        _ => None,
    }
}

/// The password manager setting a known manager's extension connects
/// (`madoConnect`, 09 §8): `passwords.bitwarden` or `passwords.onePassword`.
fn manager_setting(id: &ExtensionId) -> Option<&'static str> {
    match mado_lists::compat()
        .extensions
        .get(id.as_str())?
        .manager
        .as_deref()?
    {
        "bitwarden" => Some("passwords.bitwarden"),
        "1password" => Some("passwords.onePassword"),
        _ => None,
    }
}

fn choices(category: &str, id: &ExtensionId) -> Vec<OverlapChoice> {
    use OverlapChoice::*;
    match category {
        "passwordManager" if manager_setting(id).is_some() => {
            vec![ExtensionOnly, Both, MadoOnly, MadoConnect]
        }
        "adBlocker" | "passwordManager" => vec![ExtensionOnly, Both, MadoOnly],
        _ => vec![ExtensionOnly, MadoOnly],
    }
}

/// The settings a choice changes (09 §8 table). Dark pages and translation
/// have no setting before v1.1, so those choices change nothing.
fn effects(category: &str, choice: OverlapChoice, id: &ExtensionId) -> Vec<(&'static str, Value)> {
    match (category, choice) {
        ("passwordManager", OverlapChoice::MadoConnect) => {
            // Mado fills from every source, the manager's CLI among them;
            // the extension stays on for its own window.
            let mut v = vec![("passwords.whoFills", json!("automatic"))];
            if let Some(key) = manager_setting(id) {
                v.push((key, json!(true)));
            }
            v
        }
        ("adBlocker", OverlapChoice::ExtensionOnly) => vec![("blocker.enabled", json!(false))],
        ("passwordManager", OverlapChoice::ExtensionOnly) => {
            vec![("passwords.whoFills", json!("extension"))]
        }
        ("passwordManager", OverlapChoice::Both) => {
            vec![("passwords.whoFills", json!("automatic"))]
        }
        ("tabSleeping", OverlapChoice::ExtensionOnly) => vec![("tabs.memorySaver", json!("off"))],
        _ => Vec::new(),
    }
}

impl Core {
    /// Sets `key` as Mado itself and returns its value before.
    fn system_set(&mut self, key: &str, value: Value, rec: &mut PatchRecorder) -> Option<Value> {
        let def = mado_settings::lookup(key)?;
        let before = self.settings.value(def);
        let now = self.now();
        match self
            .settings
            .set(key, value, &Origin::System, None, self.tokens.at(now), now)
        {
            Ok(out) => {
                self.apply_outcome(out, rec);
                Some(before)
            }
            Err(e) => {
                tracing::warn!(key, error = ?e, "an overlap setting was not changed");
                None
            }
        }
    }

    fn overlap_notice(&mut self, key: &str, name: &str, rec: &mut PatchRecorder) {
        let id = format!("n{}", self.next_notice);
        self.next_notice += 1;
        let notice = Notice {
            id,
            message_key: key.into(),
            args: BTreeMap::from([("name".to_owned(), json!(name))]),
            action: None,
            undo: None,
            expires_at: Some(self.now() + 10_000),
        };
        self.add_notice(notice, rec);
    }

    fn add_ext_prompt(&mut self, prompt: PromptState, rec: &mut PatchRecorder) {
        let mut prompts = self.state.prompts.clone();
        let id = match &prompt {
            PromptState::Overlap { id, .. } | PromptState::Takeover { id, .. } => *id,
            _ => return,
        };
        prompts.insert(id, prompt);
        self.state.tx(rec).set_prompts(prompts);
    }

    /// An extension was installed or turned on (09 §8).
    pub(super) fn extension_turned_on(&mut self, id: &ExtensionId, rec: &mut PatchRecorder) {
        let Some(info) = self.extension_info.get(id).cloned() else {
            return;
        };
        let name = self
            .state
            .extensions
            .installed
            .iter()
            .find(|e| &e.id == id)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        if let Some(category) = category_of(id, info.dnr_rules)
            && self.setting("extensions.overlapPrompts") != false
            && !self
                .overlaps
                .get(category)
                .is_some_and(|r| r.extension_id.as_ref() == Some(id))
        {
            let choices = choices(category, id);
            self.add_ext_prompt(
                PromptState::Overlap {
                    id: PromptId::new(),
                    category: category.into(),
                    extension_id: id.clone(),
                    name: name.clone(),
                    choices,
                    default: OverlapChoice::ExtensionOnly,
                },
                rec,
            );
        }
        let mut whats = Vec::new();
        if info.new_tab.is_some() {
            whats.push("newTab");
        }
        if let Some(search) = &info.search {
            let engine = mado_navigation::engines::Engine {
                id: format!("ext-{id}"),
                name: search.name.clone(),
                keyword: search.keyword.clone(),
                search_url: search.search_url.clone(),
                suggest_url: search.suggest_url.clone(),
            };
            self.add_extension_engine(engine);
            let previous = self.system_set("search.defaultEngine", json!(format!("ext-{id}")), rec);
            self.takeover_applied(
                id,
                "search",
                previous
                    .map(|v| ("search.defaultEngine", v))
                    .into_iter()
                    .collect(),
            );
            whats.push("search");
        }
        if !info.startup_pages.is_empty() {
            let mode = self.system_set("startup.mode", json!("pages"), rec);
            let pages = self.system_set("startup.pages", json!(info.startup_pages), rec);
            let previous = [("startup.mode", mode), ("startup.pages", pages)]
                .into_iter()
                .filter_map(|(k, v)| v.map(|v| (k, v)))
                .collect();
            self.takeover_applied(id, "startup", previous);
            whats.push("startup");
        }
        self.refresh_controlled(rec);
        for what in whats {
            let key = (id.clone(), what.to_owned());
            if self.takeovers.get(&key).is_some_and(|r| r.kept) {
                continue;
            }
            self.takeovers.entry(key).or_insert_with(|| TakeoverRow {
                kept: false,
                previous: BTreeMap::new(),
            });
            self.add_ext_prompt(
                PromptState::Takeover {
                    id: PromptId::new(),
                    extension_id: id.clone(),
                    name: name.clone(),
                    what: what.into(),
                },
                rec,
            );
        }
    }

    /// Records what a takeover changed, keeping the oldest values.
    fn takeover_applied(&mut self, id: &ExtensionId, what: &str, previous: Vec<(&str, Value)>) {
        let row = self
            .takeovers
            .entry((id.clone(), what.to_owned()))
            .or_insert_with(|| TakeoverRow {
                kept: false,
                previous: BTreeMap::new(),
            });
        for (k, v) in previous {
            row.previous.entry(k.to_owned()).or_insert(v);
        }
        self.save_takeover(id, what);
    }

    fn save_takeover(&mut self, id: &ExtensionId, what: &str) {
        let Some(row) = self.takeovers.get(&(id.clone(), what.to_owned())).cloned() else {
            return;
        };
        let (id, what, now) = (id.to_string(), what.to_owned(), self.now());
        let previous = json!(row.previous).to_string();
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "INSERT INTO extension_takeovers (extension_id, what, kept, previous, decided_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(extension_id, what) DO UPDATE SET kept = ?3, previous = ?4, decided_at = ?5",
                params![id, what, row.kept, previous, now],
            )
            .map(|_| ())
        });
        Self::log_write(pending);
    }

    fn log_write(pending: mado_storage::PendingWrite<()>) {
        tokio::task::spawn_local(async move {
            if let Err(e) = pending.await {
                tracing::error!(error = %e, "an overlap change was not saved");
            }
        });
    }

    fn add_extension_engine(&mut self, engine: mado_navigation::engines::Engine) {
        self.engines.retain(|e| e.engine.id != engine.id);
        self.engines.push(super::nav::EngineRow {
            engine: engine.clone(),
            active: true,
            source: "extension".into(),
        });
        let now = self.now();
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO search_engines (id, name, keyword, search_url, suggest_url, active, source, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, 'extension', ?6)",
                params![engine.id, engine.name, engine.keyword, engine.search_url, engine.suggest_url, now],
            )
            .map(|_| ())
        });
        Self::log_write(pending);
    }

    /// An extension was turned off or removed: what its overlap choice and
    /// takeovers changed goes back, with a notice (09 §8).
    pub(super) fn extension_turned_off(
        &mut self,
        id: &ExtensionId,
        removed: bool,
        rec: &mut PatchRecorder,
    ) {
        let name = self
            .state
            .extensions
            .installed
            .iter()
            .find(|e| &e.id == id)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        let mut restored = false;
        let categories: Vec<String> = self
            .overlaps
            .iter()
            .filter(|(_, r)| {
                r.extension_id.as_ref() == Some(id)
                    && (removed || r.choice != OverlapChoice::MadoOnly)
            })
            .map(|(c, _)| c.clone())
            .collect();
        for category in categories {
            if let Some(row) = self.overlaps.remove(&category) {
                for (k, v) in row.previous {
                    restored |= self.system_set(&k, v, rec).is_some();
                }
                let pending = self.profile_db.writer().write(move |c| {
                    c.execute("DELETE FROM overlaps WHERE category = ?1", [category])
                        .map(|_| ())
                });
                Self::log_write(pending);
            }
        }
        let keys: Vec<(ExtensionId, String)> = self
            .takeovers
            .keys()
            .filter(|(e, _)| e == id)
            .cloned()
            .collect();
        for key in keys {
            if let Some(row) = self.takeovers.remove(&key) {
                for (k, v) in row.previous {
                    restored |= self.system_set(&k, v, rec).is_some();
                }
            }
        }
        if self
            .engines
            .iter()
            .any(|e| e.engine.id == format!("ext-{id}"))
        {
            let engine = format!("ext-{id}");
            self.engines.retain(|e| e.engine.id != engine);
            let pending = self.profile_db.writer().write(move |c| {
                c.execute("DELETE FROM search_engines WHERE id = ?1", [engine])
                    .map(|_| ())
            });
            Self::log_write(pending);
        }
        let ext = id.to_string();
        let pending = self.profile_db.writer().write(move |c| {
            c.execute(
                "DELETE FROM extension_takeovers WHERE extension_id = ?1",
                [ext],
            )
            .map(|_| ())
        });
        Self::log_write(pending);
        let mut prompts = self.state.prompts.clone();
        prompts.retain(|_, p| {
            !matches!(p, PromptState::Overlap { extension_id, .. } | PromptState::Takeover { extension_id, .. } if extension_id == id)
        });
        if prompts != self.state.prompts {
            self.state.tx(rec).set_prompts(prompts);
        }
        self.refresh_controlled(rec);
        if restored {
            self.overlap_notice("notice.overlapRestored", &name, rec);
        }
    }

    /// `state.extensions.controlled` from the overlap choices and takeovers.
    pub(super) fn refresh_controlled(&mut self, rec: &mut PatchRecorder) {
        let mut controlled = BTreeMap::new();
        for row in self.overlaps.values() {
            if let Some(id) = &row.extension_id
                && row.choice != OverlapChoice::MadoOnly
            {
                for k in row.previous.keys() {
                    controlled.insert(k.clone(), id.clone());
                }
            }
        }
        for ((id, _), row) in &self.takeovers {
            for k in row.previous.keys() {
                controlled.insert(k.clone(), id.clone());
            }
        }
        if controlled != self.state.extensions.controlled {
            self.state.tx(rec).extensions().set_controlled(controlled);
        }
    }

    /// The new tab page: an enabled extension's override, else Mado's.
    pub(super) fn new_tab_url(&self) -> String {
        self.state
            .extensions
            .installed
            .iter()
            .filter(|e| e.enabled)
            .find_map(|e| {
                let info = self.extension_info.get(&e.id)?;
                let page = info.new_tab.as_ref()?;
                let base = info.base_url.as_ref()?;
                Some(format!(
                    "{}/{}",
                    base.trim_end_matches('/'),
                    page.trim_start_matches('/')
                ))
            })
            .unwrap_or_else(|| super::session::NEW_TAB.to_owned())
    }

    fn take_prompt(
        &mut self,
        id: PromptId,
        rec: &mut PatchRecorder,
    ) -> Result<PromptState, MadoError> {
        let mut prompts = self.state.prompts.clone();
        let prompt = prompts
            .remove(&id)
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.prompt.notFound"))?;
        self.state.tx(rec).set_prompts(prompts);
        Ok(prompt)
    }

    /// `extension.overlapRespond`, `takeoverRespond`, `overlaps`,
    /// `overlapReset`.
    pub(super) fn overlap_command(
        &mut self,
        origin: &Origin,
        cmd: Command,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        if !matches!(cmd, Command::ExtensionOverlaps) && !origin.is_user() {
            return Err(MadoError::new(
                ErrorCode::Protected,
                "error.extension.notAllowed",
            ));
        }
        match cmd {
            Command::ExtensionOverlapRespond { prompt_id, choice } => {
                let prompt = match self.state.prompts.get(&prompt_id) {
                    Some(p @ PromptState::Overlap { .. }) => p.clone(),
                    _ => return Err(MadoError::new(ErrorCode::NotFound, "error.prompt.notFound")),
                };
                let PromptState::Overlap { category, extension_id, choices, .. } = prompt else { unreachable!() };
                if !choices.contains(&choice) {
                    return Err(MadoError::new(ErrorCode::InvalidInput, "error.extension.badChoice"));
                }
                self.take_prompt(prompt_id, rec)?;
                let mut previous = BTreeMap::new();
                for (key, value) in effects(&category, choice, &extension_id) {
                    if let Some(before) = self.system_set(key, value, rec) {
                        previous.insert(key.to_owned(), before);
                    }
                }
                self.overlaps.insert(
                    category.clone(),
                    OverlapRow { choice, extension_id: Some(extension_id.clone()), previous: previous.clone() },
                );
                let (ext, now, prev) = (extension_id.to_string(), self.now(), json!(previous).to_string());
                let pending = self.profile_db.writer().write(move |c| {
                    c.execute(
                        "INSERT OR REPLACE INTO overlaps (category, choice, extension_id, decided_at, previous) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![category, choice.column(), ext, now, prev],
                    )
                    .map(|_| ())
                });
                Self::log_write(pending);
                self.refresh_controlled(rec);
                if choice == OverlapChoice::MadoOnly
                    && let Some(mut e) = self.state.extensions.installed.iter().find(|e| e.id == extension_id).cloned()
                {
                    e.enabled = false;
                    self.save_installed(e, true, rec);
                }
                Ok(Value::Null)
            }
            Command::ExtensionTakeoverRespond { prompt_id, keep } => {
                let PromptState::Takeover { extension_id, what, .. } = self.take_prompt(prompt_id, rec)? else {
                    return Err(MadoError::new(ErrorCode::NotFound, "error.prompt.notFound"));
                };
                if keep {
                    if let Some(row) = self.takeovers.get_mut(&(extension_id.clone(), what.clone())) {
                        row.kept = true;
                    }
                    self.save_takeover(&extension_id, &what);
                } else if let Some(mut e) = self.state.extensions.installed.iter().find(|e| e.id == extension_id).cloned() {
                    // Change it back: the extension turns off.
                    e.enabled = false;
                    self.save_installed(e, true, rec);
                }
                Ok(Value::Null)
            }
            Command::ExtensionOverlaps => Ok(json!(self
                .overlaps
                .iter()
                .map(|(category, r)| {
                    let name = r.extension_id.as_ref().and_then(|id| {
                        self.state.extensions.installed.iter().find(|e| &e.id == id).map(|e| e.name.clone())
                    });
                    json!({ "category": category, "choice": r.choice, "extensionId": r.extension_id, "name": name })
                })
                .collect::<Vec<_>>())),
            Command::ExtensionOverlapReset { category } => {
                let Some(row) = self.overlaps.get(&category).cloned() else {
                    return Err(MadoError::new(ErrorCode::NotFound, "error.extension.noOverlap"));
                };
                for (k, v) in row.previous {
                    self.system_set(&k, v, rec);
                }
                self.overlaps.remove(&category);
                let c2 = category.clone();
                let pending = self
                    .profile_db
                    .writer()
                    .write(move |c| c.execute("DELETE FROM overlaps WHERE category = ?1", [c2]).map(|_| ()));
                Self::log_write(pending);
                self.refresh_controlled(rec);
                if let Some(id) = row.extension_id
                    && self.state.extensions.installed.iter().any(|e| e.id == id && e.enabled)
                {
                    self.extension_turned_on(&id, rec);
                }
                Ok(Value::Null)
            }
            _ => Err(MadoError::new(ErrorCode::InvalidInput, "error.command.unknown")),
        }
    }
}

#[cfg(test)]
mod password_manager_tests {
    use super::*;

    /// 09 §8: a known password manager's extension can be connected to
    /// Mado instead (`madoConnect`): Mado fills through its CLI.
    #[test]
    fn a_known_manager_offers_mado_connect() {
        let bitwarden: ExtensionId = "nngceckbapebfimnlniiiahkandclblb".parse().expect("id");
        let other: ExtensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().expect("id");
        assert!(choices("passwordManager", &bitwarden).contains(&OverlapChoice::MadoConnect));
        assert!(!choices("passwordManager", &other).contains(&OverlapChoice::MadoConnect));
        assert_eq!(
            effects("passwordManager", OverlapChoice::MadoConnect, &bitwarden),
            vec![
                ("passwords.whoFills", json!("automatic")),
                ("passwords.bitwarden", json!(true))
            ]
        );
        assert_eq!(
            effects("passwordManager", OverlapChoice::ExtensionOnly, &bitwarden),
            vec![("passwords.whoFills", json!("extension"))]
        );
    }
}
