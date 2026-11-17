//! The model catalogue and roles (spec 13 §4): what each connection's
//! models are, which a picker shows first, and which model a role uses.

use std::collections::BTreeMap;

use mado_lists::{ModelInfo, Models, ROLES};
use serde::{Deserialize, Serialize};

/// A model one connection offers, as `ai_models` keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    pub connection: String,
    pub provider: String,
    pub model: String,
    /// The name the provider gave, if any.
    pub name: Option<String>,
    pub show: bool,
    pub starred: bool,
    /// The connection is ready and its priority (lower first).
    pub ready: bool,
    pub priority: i64,
}

impl Available {
    /// `<connectionId>:<modelId>`, how roles name a model.
    pub fn reference(&self) -> String {
        format!("{}:{}", self.connection, self.model)
    }
}

/// The catalogue entry for a provider's model. OpenRouter's IDs are
/// already `vendor/model`, with a `:variant` sometimes.
pub fn known<'a>(models: &'a Models, provider: &str, model: &str) -> Option<&'a ModelInfo> {
    models
        .models
        .get(&format!("{provider}/{model}"))
        .or_else(|| {
            (provider == "openrouter").then(|| models.models.get(model.split(':').next()?))?
        })
        .or_else(|| {
            // Dated snapshots (`claude-sonnet-4-5-20250929`) and `-latest`.
            let base = model
                .rsplit_once('-')
                .filter(|(_, tail)| {
                    tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) || *tail == "latest"
                })?
                .0;
            models.models.get(&format!("{provider}/{base}"))
        })
}

/// One row of a model picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickerModel {
    pub connection_id: String,
    pub model_id: String,
    pub name: String,
    pub show: bool,
    pub starred: bool,
    /// `None`: "unknown price".
    pub info: Option<ModelInfo>,
}

/// 13 §4: starred first, then shown models grouped by connection (in
/// priority order), each by name; hidden ones last.
pub fn picker(models: &Models, available: &[Available]) -> Vec<PickerModel> {
    let mut rows: Vec<(&Available, PickerModel)> = available
        .iter()
        .map(|a| {
            let info = known(models, &a.provider, &a.model).cloned();
            let name = info
                .as_ref()
                .map(|i| i.name.clone())
                .or_else(|| a.name.clone())
                .unwrap_or_else(|| a.model.clone());
            (
                a,
                PickerModel {
                    connection_id: a.connection.clone(),
                    model_id: a.model.clone(),
                    name,
                    show: a.show,
                    starred: a.starred,
                    info,
                },
            )
        })
        .collect();
    rows.sort_by(|(a, x), (b, y)| {
        (
            !x.starred,
            !x.show,
            a.priority,
            &a.connection,
            x.name.to_lowercase(),
        )
            .cmp(&(
                !y.starred,
                !y.show,
                b.priority,
                &b.connection,
                y.name.to_lowercase(),
            ))
    });
    rows.into_iter().map(|(_, p)| p).collect()
}

/// A role's choice (`ai_roles`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleChoice {
    /// `auto` or `<connectionId>:<modelId>`.
    pub model: String,
    /// `auto`, `low`, `medium`, `high`, `xhigh` or `max`.
    pub effort: String,
    /// Another model reference used when `model` fails.
    pub fallback: Option<String>,
}

impl RoleChoice {
    pub fn auto() -> Self {
        Self {
            model: "auto".into(),
            effort: "auto".into(),
            fallback: None,
        }
    }
}

/// How a model sorts for a role: lower first.
type SortKey = (bool, u32, u8, bool, i64);

/// Whether a known model can do the role. Models that cannot still come
/// last, so every role resolves with any connection; callers leave out
/// what the model does not take (images for a model without vision).
fn capable(role: &str, info: Option<&ModelInfo>) -> bool {
    let Some(i) = info else { return true };
    match role {
        "vision" => i.vision,
        "tasks" => i.tools,
        "image" => true,
        _ => i.max_output > 0,
    }
}

/// The ready, shown models for `role`, best first: capable before not,
/// then ranked models by rank, then unranked known ones, then unknown ones; ties prefer
/// `family` (the `tasks` role's provider), then the connection's priority.
fn ranked<'a>(
    models: &Models,
    available: &'a [Available],
    role: &str,
    family: Option<&str>,
) -> Vec<&'a Available> {
    let mut list: Vec<(&Available, SortKey)> = available
        .iter()
        .filter(|a| a.ready && a.show)
        .map(|a| {
            let info = known(models, &a.provider, &a.model);
            let (rank, tier) = match info {
                Some(i) => i.roles.get(role).map_or((u32::MAX, 1), |r| (*r, 0)),
                None => (u32::MAX, 2),
            };
            let vendor = info.map(|_| a.provider.as_str());
            let key = (
                !capable(role, info),
                rank,
                tier,
                family.is_some() && vendor != family,
                a.priority,
            );
            (a, key)
        })
        .collect();
    list.sort_by(|(a, x), (b, y)| x.cmp(y).then_with(|| a.reference().cmp(&b.reference())));
    list.into_iter().map(|(a, _)| a).collect()
}

/// The model a role uses now (13 §4): its own choice while that model is
/// available, else its fallback, else the best for the role. `None` when
/// no ready connection offers a model.
pub fn resolve(
    models: &Models,
    available: &[Available],
    roles: &BTreeMap<String, RoleChoice>,
    role: &str,
) -> Option<String> {
    let usable = |r: &str| available.iter().any(|a| a.ready && a.reference() == r);
    let choice = roles.get(role).cloned().unwrap_or_else(RoleChoice::auto);
    if choice.model != "auto" && usable(&choice.model) {
        return Some(choice.model);
    }
    if let Some(f) = choice.fallback.as_deref().filter(|f| usable(f)) {
        return Some(f.to_owned());
    }
    // Ties go to the provider of the model tasks use.
    let family = if role == "tasks" {
        None
    } else {
        roles
            .get("tasks")
            .filter(|c| c.model != "auto")
            .and_then(|c| available.iter().find(|a| a.reference() == c.model))
            .or_else(|| ranked(models, available, "tasks", None).first().copied())
            .map(|a| a.provider.clone())
    };
    ranked(models, available, role, family.as_deref())
        .first()
        .map(|a| a.reference())
}

/// Every role's model.
pub fn resolve_all(
    models: &Models,
    available: &[Available],
    roles: &BTreeMap<String, RoleChoice>,
) -> BTreeMap<String, Option<String>> {
    ROLES
        .iter()
        .map(|r| ((*r).to_owned(), resolve(models, available, roles, r)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn avail(connection: &str, provider: &str, model: &str, priority: i64) -> Available {
        Available {
            connection: connection.into(),
            provider: provider.into(),
            model: model.into(),
            name: None,
            show: true,
            starred: false,
            ready: true,
            priority,
        }
    }

    /// The M5-T03 check: with any single connection every role resolves.
    #[test]
    fn every_role_resolves_with_any_single_connection() {
        let models = mado_lists::models();
        let singles = [
            vec![
                avail("a", "anthropic", "claude-sonnet-4-5-20250929", 0),
                avail("a", "anthropic", "claude-haiku-4-5", 0),
            ],
            vec![
                avail("o", "openai", "gpt-5", 0),
                avail("o", "openai", "whisper-1", 0),
            ],
            vec![avail("g", "google", "gemini-2.5-flash", 0)],
            vec![avail("x", "xai", "grok-4", 0)],
            vec![avail("m", "mistral", "mistral-large-latest", 0)],
            vec![avail("d", "deepseek", "deepseek-chat", 0)],
            vec![avail("q", "groq", "llama-3.3-70b-versatile", 0)],
            vec![avail("r", "openrouter", "anthropic/claude-sonnet-4-5", 0)],
            vec![avail("l", "ollama", "llama3.2:latest", 0)],
            vec![avail("s", "lmStudio", "qwen2.5-7b-instruct", 0)],
            vec![avail("c", "openaiCompatible", "my-model", 0)],
        ];
        for single in singles {
            let resolved = resolve_all(&models, &single, &BTreeMap::new());
            for (role, model) in &resolved {
                assert!(model.is_some(), "{role} with {:?}", single[0].provider);
            }
        }
        assert!(
            resolve_all(&models, &[], &BTreeMap::new())
                .values()
                .all(Option::is_none)
        );
    }

    #[test]
    fn auto_takes_the_best_rank_and_prefers_the_tasks_family_on_ties() {
        let models = mado_lists::models();
        let both = [
            avail("a", "anthropic", "claude-sonnet-4-5", 1),
            avail("a", "anthropic", "claude-haiku-4-5", 1),
            avail("a", "anthropic", "claude-opus-4-1", 1),
            avail("o", "openai", "gpt-5", 0),
            avail("o", "openai", "gpt-5-mini", 0),
        ];
        let r = resolve_all(&models, &both, &BTreeMap::new());
        assert_eq!(r["tasks"].as_deref(), Some("a:claude-sonnet-4-5"));
        assert_eq!(r["deep"].as_deref(), Some("a:claude-opus-4-1"));
        assert_eq!(r["fast"].as_deref(), Some("a:claude-haiku-4-5"));
        // Unranked for image: known models tie, the tasks family wins.
        assert_eq!(
            r["image"].as_deref().map(|m| m.starts_with("a:")),
            Some(true)
        );
        // A chosen model wins while it is available; then its fallback.
        let mut roles = BTreeMap::new();
        roles.insert(
            "chat".to_owned(),
            RoleChoice {
                model: "o:gpt-5-mini".into(),
                effort: "low".into(),
                fallback: None,
            },
        );
        assert_eq!(
            resolve(&models, &both, &roles, "chat").as_deref(),
            Some("o:gpt-5-mini")
        );
        roles.insert(
            "chat".to_owned(),
            RoleChoice {
                model: "z:gone".into(),
                effort: "auto".into(),
                fallback: Some("o:gpt-5".into()),
            },
        );
        assert_eq!(
            resolve(&models, &both, &roles, "chat").as_deref(),
            Some("o:gpt-5")
        );
        // Hidden and not-ready models are never picked.
        let mut hidden = both.to_vec();
        for a in &mut hidden {
            a.show = a.model != "claude-sonnet-4-5";
        }
        hidden[3].ready = false;
        assert_eq!(
            resolve(&models, &hidden, &BTreeMap::new(), "tasks").as_deref(),
            Some("a:claude-opus-4-1")
        );
    }

    #[test]
    fn capable_models_come_first() {
        let models = mado_lists::models();
        let only = [
            avail("d", "deepseek", "deepseek-reasoner", 0),
            avail("d", "deepseek", "deepseek-chat", 0),
        ];
        assert_eq!(
            resolve(&models, &only, &BTreeMap::new(), "tasks").as_deref(),
            Some("d:deepseek-chat")
        );
        let vision = [
            avail("d", "deepseek", "deepseek-chat", 0),
            avail("d", "deepseek", "zz-unknown", 0),
        ];
        assert_eq!(
            resolve(&models, &vision, &BTreeMap::new(), "vision").as_deref(),
            Some("d:zz-unknown"),
            "an unknown model may see; a known text-only one comes last"
        );
    }

    #[test]
    fn pickers_list_starred_then_shown_by_connection() {
        let models = mado_lists::models();
        let mut list = vec![
            avail("b", "openai", "gpt-5", 1),
            avail("a", "anthropic", "claude-haiku-4-5", 0),
            avail("a", "anthropic", "zz-unknown", 0),
            avail("b", "openai", "gpt-5-mini", 1),
        ];
        list[3].starred = true;
        list[1].show = false;
        let rows = picker(&models, &list);
        let ids: Vec<&str> = rows.iter().map(|r| r.model_id.as_str()).collect();
        assert_eq!(
            ids,
            ["gpt-5-mini", "zz-unknown", "gpt-5", "claude-haiku-4-5"]
        );
        assert!(rows[1].info.is_none(), "unknown price");
        assert_eq!(rows[2].name, "GPT-5");
        assert_eq!(
            known(&models, "anthropic", "claude-haiku-4-5-20251001").map(|i| i.name.as_str()),
            Some("Claude Haiku 4.5")
        );
    }
}
