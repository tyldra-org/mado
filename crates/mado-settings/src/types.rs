//! Setting types, scopes and sections (spec 06 §1), and value validation.

use serde_json::Value;

/// Where a value is stored (spec 06 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// `app.sqlite`: one value for every profile.
    App,
    /// `profile.sqlite`.
    Profile,
    /// `site_settings` (spec 07): the registry holds only the default.
    Site,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Profile => "profile",
            Self::Site => "site",
        }
    }
}

/// Settings window sections, in page order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Section {
    General,
    Appearance,
    Tabs,
    Search,
    Privacy,
    SiteSettings,
    Passwords,
    Extensions,
    Downloads,
    Ai,
    Developers,
    Advanced,
    About,
    Features,
}

impl Section {
    pub const ALL: [Section; 14] = [
        Self::General,
        Self::Appearance,
        Self::Tabs,
        Self::Search,
        Self::Privacy,
        Self::SiteSettings,
        Self::Passwords,
        Self::Extensions,
        Self::Downloads,
        Self::Ai,
        Self::Developers,
        Self::Advanced,
        Self::About,
        Self::Features,
    ];

    /// The id in `mado://settings/<section>` and in `en.json` keys.
    pub fn id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Tabs => "tabs",
            Self::Search => "search",
            Self::Privacy => "privacy",
            Self::SiteSettings => "siteSettings",
            Self::Passwords => "passwords",
            Self::Extensions => "extensions",
            Self::Downloads => "downloads",
            Self::Ai => "ai",
            Self::Developers => "developers",
            Self::Advanced => "advanced",
            Self::About => "about",
            Self::Features => "features",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }
}

/// What a change needs before it takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    None,
    App,
    Engine,
}

/// A setting's type. `Duration` values are milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingType {
    Bool,
    Int {
        min: i64,
        max: i64,
        step: i64,
    },
    Float {
        min: f64,
        max: f64,
    },
    String {
        max_len: usize,
    },
    Enum(&'static [&'static str]),
    List(&'static SettingType),
    Url,
    /// An accelerator such as `Cmd+Shift+T`.
    Shortcut,
    Duration {
        min: i64,
        max: i64,
    },
    Path,
    /// Any JSON value; `schema` is the JSON Schema it must meet.
    Json {
        schema: &'static str,
    },
}

impl SettingType {
    /// The value, if it has this type. Integral floats count as integers.
    pub fn validate(&self, value: &Value) -> Result<(), String> {
        match (self, value) {
            (Self::Bool, Value::Bool(_)) => Ok(()),
            (Self::Int { min, max, step }, v) => {
                let n = v.as_i64().ok_or("not an integer")?;
                if n < *min || n > *max {
                    return Err(format!("outside {min}..={max}"));
                }
                if *step > 1 && (n - min) % step != 0 {
                    return Err(format!("not a multiple of {step}"));
                }
                Ok(())
            }
            (Self::Duration { min, max }, v) => {
                let n = v.as_i64().ok_or("not milliseconds")?;
                (n >= *min && n <= *max)
                    .then_some(())
                    .ok_or_else(|| format!("outside {min}..={max} ms"))
            }
            (Self::Float { min, max }, v) => {
                let n = v.as_f64().ok_or("not a number")?;
                (n >= *min && n <= *max)
                    .then_some(())
                    .ok_or_else(|| format!("outside {min}..={max}"))
            }
            (Self::String { max_len }, Value::String(s)) => (s.chars().count() <= *max_len)
                .then_some(())
                .ok_or_else(|| format!("longer than {max_len}")),
            (Self::Enum(options), Value::String(s)) => options
                .contains(&s.as_str())
                .then_some(())
                .ok_or_else(|| format!("not one of {options:?}")),
            (Self::List(item), Value::Array(items)) => {
                items.iter().try_for_each(|v| item.validate(v))
            }
            (Self::Url, Value::String(s)) => {
                valid_url(s).then_some(()).ok_or_else(|| "not a URL".into())
            }
            (Self::Shortcut, Value::String(s)) => valid_shortcut(s)
                .then_some(())
                .ok_or_else(|| "not a shortcut".into()),
            (Self::Path, Value::String(s)) => {
                (s.is_empty() || s.starts_with('/') || s.starts_with("~/"))
                    .then_some(())
                    .ok_or_else(|| "not an absolute path".into())
            }
            (Self::Json { .. }, _) => Ok(()),
            _ => Err("wrong type".into()),
        }
    }

    /// A short name of the type, for the settings page's row renderer.
    pub fn renderer(&self) -> &'static str {
        match self {
            Self::Bool => "toggle",
            Self::Int { .. } | Self::Float { .. } => "number",
            Self::String { .. } => "text",
            Self::Enum(_) => "select",
            Self::List(_) => "list",
            Self::Url => "url",
            Self::Shortcut => "shortcut",
            Self::Duration { .. } => "duration",
            Self::Path => "folder",
            Self::Json { .. } => "json",
        }
    }

    /// The TypeScript type of values.
    pub fn typescript(&self) -> String {
        match self {
            Self::Bool => "boolean".into(),
            Self::Int { .. } | Self::Float { .. } | Self::Duration { .. } => "number".into(),
            Self::Enum(options) => options
                .iter()
                .map(|o| format!("{o:?}"))
                .collect::<Vec<_>>()
                .join(" | "),
            Self::List(item) => format!("Array<{}>", item.typescript()),
            Self::String { .. } | Self::Url | Self::Shortcut | Self::Path => "string".into(),
            Self::Json { .. } => "unknown".into(),
        }
    }

    /// A JSON Schema fragment, for `settings.import` and MCP tool
    /// descriptions (spec 06 §1).
    pub fn json_schema(&self) -> Value {
        use serde_json::json;
        match self {
            Self::Bool => json!({ "type": "boolean" }),
            Self::Int { min, max, step } => {
                json!({ "type": "integer", "minimum": min, "maximum": max, "multipleOf": step })
            }
            Self::Float { min, max } => json!({ "type": "number", "minimum": min, "maximum": max }),
            Self::Duration { min, max } => {
                json!({ "type": "integer", "minimum": min, "maximum": max, "description": "milliseconds" })
            }
            Self::String { max_len } => json!({ "type": "string", "maxLength": max_len }),
            Self::Enum(options) => json!({ "type": "string", "enum": options }),
            Self::List(item) => json!({ "type": "array", "items": item.json_schema() }),
            Self::Url => json!({ "type": "string", "format": "uri" }),
            Self::Shortcut => {
                json!({ "type": "string", "description": "an accelerator such as Cmd+Shift+T" })
            }
            Self::Path => json!({ "type": "string", "description": "an absolute path or ~/..." }),
            Self::Json { schema } => serde_json::from_str(schema).unwrap_or(Value::Bool(true)),
        }
    }
}

fn valid_url(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    !rest.is_empty()
        && scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !s.chars().any(char::is_whitespace)
}

const MODIFIERS: [&str; 4] = ["Cmd", "Ctrl", "Alt", "Shift"];

/// `Cmd+Shift+T`: modifiers once each, then one key. An empty string
/// means no shortcut.
fn valid_shortcut(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let parts: Vec<&str> = s.split('+').collect();
    let Some((key, mods)) = parts.split_last() else {
        return false;
    };
    let mut seen = Vec::new();
    for m in mods {
        if !MODIFIERS.contains(m) || seen.contains(m) {
            return false;
        }
        seen.push(m);
    }
    !key.is_empty() && !MODIFIERS.contains(key) && !mods.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_are_checked_against_their_type() {
        assert!(
            SettingType::Int {
                min: 0,
                max: 10,
                step: 2
            }
            .validate(&json!(4))
            .is_ok()
        );
        assert!(
            SettingType::Int {
                min: 0,
                max: 10,
                step: 2
            }
            .validate(&json!(3))
            .is_err()
        );
        assert!(
            SettingType::Enum(&["a", "b"])
                .validate(&json!("c"))
                .is_err()
        );
        assert!(
            SettingType::List(&SettingType::Url)
                .validate(&json!(["https://x.test/", "mado://newtab"]))
                .is_ok()
        );
        assert!(
            SettingType::List(&SettingType::Url)
                .validate(&json!(["not a url"]))
                .is_err()
        );
        assert!(
            SettingType::Shortcut
                .validate(&json!("Cmd+Shift+T"))
                .is_ok()
        );
        assert!(SettingType::Shortcut.validate(&json!("Cmd+Cmd+T")).is_err());
        assert!(SettingType::Shortcut.validate(&json!("T")).is_err());
        assert!(SettingType::Bool.validate(&json!("true")).is_err());
        assert!(SettingType::Path.validate(&json!("relative/dir")).is_err());
    }
}
