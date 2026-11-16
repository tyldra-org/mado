//! Item JSON (spec 11 §2.2), stored encrypted in `items.data`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Login,
    Passkey,
    Card,
    Address,
    Identity,
    Note,
    Totp,
}

impl ItemKind {
    /// The `items.kind` column.
    pub fn column(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Passkey => "passkey",
            Self::Card => "card",
            Self::Address => "address",
            Self::Identity => "identity",
            Self::Note => "note",
            Self::Totp => "totp",
        }
    }

    pub fn from_column(s: &str) -> Option<Self> {
        Some(match s {
            "login" => Self::Login,
            "passkey" => Self::Passkey,
            "card" => Self::Card,
            "address" => Self::Address,
            "identity" => Self::Identity,
            "note" => Self::Note,
            "totp" => Self::Totp,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordChange {
    pub password: String,
    pub changed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    pub title: String,
    pub urls: Vec<String>,
    pub username: String,
    pub password: String,
    /// An `otpauth://` URI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub password_history: Vec<PasswordChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Passkey {
    pub rp_id: String,
    /// base64url.
    pub credential_id: String,
    pub user_handle: String,
    pub user_name: String,
    pub user_display_name: String,
    /// PKCS#8, base64.
    pub private_key: String,
    pub algorithm: i64,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_id: Option<String>,
    /// The signature counter Mado last reported (always 0 today).
    #[serde(default)]
    pub sign_count: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub name_on_card: String,
    pub number: String,
    pub exp_month: u32,
    pub exp_year: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cvc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    pub lines: Vec<String>,
    pub city: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub postal_code: String,
    pub country: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub given_name: String,
    pub family_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birthday: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub title: String,
    pub body: String,
}

/// A code not attached to a login (spec 11 §7).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Totp {
    pub title: String,
    /// An `otpauth://` URI.
    pub uri: String,
    #[serde(default)]
    pub urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Item {
    Login(Login),
    Passkey(Passkey),
    Card(Card),
    Address(Address),
    Identity(Identity),
    Note(Note),
    Totp(Totp),
}

/// The registrable domain of a URL or host, lowercased.
pub fn site_of(url_or_host: &str) -> Option<String> {
    let host = match url::Url::parse(url_or_host) {
        Ok(u) => u.host_str()?.to_owned(),
        Err(_) => url_or_host.trim().trim_end_matches('/').to_owned(),
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(psl::domain_str(&host).map_or(host.clone(), str::to_owned))
}

impl Item {
    pub fn kind(&self) -> ItemKind {
        match self {
            Self::Login(_) => ItemKind::Login,
            Self::Passkey(_) => ItemKind::Passkey,
            Self::Card(_) => ItemKind::Card,
            Self::Address(_) => ItemKind::Address,
            Self::Identity(_) => ItemKind::Identity,
            Self::Note(_) => ItemKind::Note,
            Self::Totp(_) => ItemKind::Totp,
        }
    }

    /// What `item_index` tags: `site:<registrable domain>` per URL and
    /// `rp:<rpId>` for passkeys.
    pub fn index_texts(&self) -> Vec<String> {
        let sites = |urls: &[String]| -> Vec<String> {
            let mut v: Vec<String> = urls
                .iter()
                .filter_map(|u| site_of(u))
                .map(|s| format!("site:{s}"))
                .collect();
            v.sort();
            v.dedup();
            v
        };
        match self {
            Self::Login(l) => sites(&l.urls),
            Self::Totp(t) => sites(&t.urls),
            Self::Passkey(p) => {
                let mut v = vec![format!("rp:{}", p.rp_id.to_ascii_lowercase())];
                if let Some(s) = site_of(&p.rp_id) {
                    v.push(format!("site:{s}"));
                }
                v
            }
            _ => Vec::new(),
        }
    }

    /// A short name for lists.
    pub fn title(&self) -> String {
        match self {
            Self::Login(l) => {
                if l.title.is_empty() {
                    l.urls.first().and_then(|u| site_of(u)).unwrap_or_default()
                } else {
                    l.title.clone()
                }
            }
            Self::Passkey(p) => p.rp_id.clone(),
            Self::Card(c) => c.nickname.clone().unwrap_or_else(|| {
                let last: String = c
                    .number
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                format!("•••• {last}")
            }),
            Self::Address(a) => a.name.clone(),
            Self::Identity(i) => format!("{} {}", i.given_name, i.family_name)
                .trim()
                .to_owned(),
            Self::Note(n) => n.title.clone(),
            Self::Totp(t) => t.title.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_round_trip_as_tagged_json() {
        let login = Item::Login(Login {
            title: "Mail".into(),
            urls: vec!["https://mail.example.co.uk/login".into()],
            username: "me".into(),
            password: "pw".into(),
            ..Login::default()
        });
        let json = serde_json::to_value(&login).expect("json");
        assert_eq!(json["kind"], "login");
        assert_eq!(json["passwordHistory"], serde_json::json!([]));
        assert_eq!(serde_json::from_value::<Item>(json).expect("back"), login);
        assert_eq!(login.index_texts(), ["site:example.co.uk"]);
    }
}
