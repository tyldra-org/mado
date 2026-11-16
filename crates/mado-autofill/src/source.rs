//! Fill sources (spec 11 §6): Mado Passwords, 1Password and Bitwarden behind
//! one interface, and the merged fill menu. Calls block (a vault read, a
//! CLI, a local server), so the core runs them on blocking threads.

use std::path::PathBuf;
use std::sync::Arc;

use mado_vault::item::{Item, ItemKind, Login, site_of};
use mado_vault::{Keys, Vault, VaultError};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceId {
    Mado,
    OnePassword,
    Bitwarden,
}

impl SourceId {
    /// The `vault.saveDestination.*` and `passwords.whoFills` value, and
    /// the prefix of an item reference (`mado:<id>`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Mado => "mado",
            Self::OnePassword => "onePassword",
            Self::Bitwarden => "bitwarden",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        [Self::Mado, Self::OnePassword, Self::Bitwarden]
            .into_iter()
            .find(|x| x.key() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceStatus {
    /// The CLI or app is not on this Mac.
    NotInstalled,
    Locked,
    Ready,
    /// Anything else, with a reason for Settings.
    Error(String),
}

/// What the fill menu shows of an item: never a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSummary {
    pub id: String,
    pub kind: ItemKind,
    pub title: String,
    pub username: String,
    /// Registrable domains the item is for.
    pub sites: Vec<String>,
    /// Unix ms, for ordering; 0 when the source does not say.
    pub last_used: i64,
}

/// A new or changed login (spec 11 §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLogin {
    pub url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("the source is locked")]
    Locked,
    #[error("no such item")]
    NotFound,
    #[error("{0}")]
    Failed(String),
}

impl From<VaultError> for SourceError {
    fn from(e: VaultError) -> Self {
        match e {
            VaultError::NotFound => Self::NotFound,
            e => Self::Failed(e.to_string()),
        }
    }
}

/// One place passwords come from (spec 11 §6).
pub trait FillSource: Send + Sync + std::fmt::Debug {
    fn id(&self) -> SourceId;
    fn status(&self) -> SourceStatus;
    /// Items for `site` (a registrable domain) of these kinds. Logins and
    /// codes by site; cards, addresses and identities whatever the site.
    fn find(&self, site: &str, kinds: &[ItemKind]) -> Result<Vec<ItemSummary>, SourceError>;
    /// The whole item, only to fill or copy it.
    fn reveal(&self, id: &str) -> Result<Item, SourceError>;
    /// The item's current one-time code.
    fn totp(&self, id: &str, now_secs: u64) -> Result<Option<String>, SourceError>;
    /// Saves a login; returns its ID.
    fn save(&self, login: &NewLogin) -> Result<String, SourceError>;
    /// Gives login `id` a new password.
    fn update_password(&self, id: &str, password: &str) -> Result<(), SourceError>;
}

/// Mado Passwords (11 §6.1), on `vault.sqlite` with the unlocked keys.
#[derive(Debug, Clone)]
pub struct MadoSource {
    pub db: PathBuf,
    pub keys: Arc<Keys>,
    /// Unix ms, for new items and `last_used_at`.
    pub now: i64,
}

impl MadoSource {
    fn conn(&self) -> Result<rusqlite::Connection, SourceError> {
        let c =
            rusqlite::Connection::open(&self.db).map_err(|e| SourceError::Failed(e.to_string()))?;
        let _ = c.busy_timeout(std::time::Duration::from_secs(5));
        let _ = c.pragma_update(None, "foreign_keys", "ON");
        Ok(c)
    }
}

fn summary(s: &mado_vault::Stored) -> ItemSummary {
    let (username, urls): (String, &[String]) = match &s.item {
        Item::Login(l) => (l.username.clone(), &l.urls),
        Item::Totp(t) => (String::new(), &t.urls),
        Item::Card(c) => {
            let last: String = c
                .number
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            (format!("•••• {last}"), &[])
        }
        Item::Address(a) => (a.lines.first().cloned().unwrap_or_default(), &[]),
        Item::Identity(i) => (i.email.clone().unwrap_or_default(), &[]),
        _ => (String::new(), &[]),
    };
    ItemSummary {
        id: s.id.clone(),
        kind: s.item.kind(),
        title: s.item.title(),
        username,
        sites: urls.iter().filter_map(|u| site_of(u)).collect(),
        last_used: s.last_used_at.unwrap_or(s.updated_at),
    }
}

impl FillSource for MadoSource {
    fn id(&self) -> SourceId {
        SourceId::Mado
    }

    fn status(&self) -> SourceStatus {
        SourceStatus::Ready
    }

    fn find(&self, site: &str, kinds: &[ItemKind]) -> Result<Vec<ItemSummary>, SourceError> {
        let c = self.conn()?;
        let by_site = kinds
            .iter()
            .any(|k| matches!(k, ItemKind::Login | ItemKind::Totp | ItemKind::Passkey));
        let mut found = if by_site {
            Vault::find(&c, &self.keys, &format!("site:{site}"))?
        } else {
            Vec::new()
        };
        if kinds
            .iter()
            .any(|k| matches!(k, ItemKind::Card | ItemKind::Address | ItemKind::Identity))
        {
            found.extend(Vault::list(&c, &self.keys, false)?);
        }
        let mut out: Vec<ItemSummary> = found
            .iter()
            .filter(|s| kinds.contains(&s.item.kind()))
            .map(summary)
            .collect();
        out.dedup_by(|a, b| a.id == b.id);
        Ok(out)
    }

    fn reveal(&self, id: &str) -> Result<Item, SourceError> {
        let c = self.conn()?;
        let item = Vault::get(&c, &self.keys, id)?.item;
        Vault::touch(&c, id, self.now)?;
        Ok(item)
    }

    fn totp(&self, id: &str, now_secs: u64) -> Result<Option<String>, SourceError> {
        Ok(match self.reveal(id)? {
            Item::Login(l) => l
                .totp
                .as_deref()
                .and_then(crate::totp::Totp::parse)
                .map(|t| t.code(now_secs)),
            Item::Totp(t) => crate::totp::Totp::parse(&t.uri).map(|t| t.code(now_secs)),
            _ => None,
        })
    }

    fn save(&self, login: &NewLogin) -> Result<String, SourceError> {
        let c = self.conn()?;
        let item = Item::Login(Login {
            title: String::new(),
            urls: vec![login.url.clone()],
            username: login.username.clone(),
            password: login.password.clone(),
            ..Login::default()
        });
        Ok(Vault::add(&c, &self.keys, &item, self.now)?)
    }

    fn update_password(&self, id: &str, password: &str) -> Result<(), SourceError> {
        let c = self.conn()?;
        let Item::Login(mut l) = Vault::get(&c, &self.keys, id)?.item else {
            return Err(SourceError::NotFound);
        };
        password.clone_into(&mut l.password);
        Ok(Vault::update(
            &c,
            &self.keys,
            id,
            &Item::Login(l),
            self.now,
        )?)
    }
}

/// One fill menu entry after merging (11 §6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    /// The source that fills it and its item.
    pub source: SourceId,
    pub summary: ItemSummary,
    /// Every source holding it, the filling one first.
    pub sources: Vec<SourceId>,
}

/// Merges the sources' items (11 §6.4): the `preferred` source first, then
/// Mado, 1Password, Bitwarden; within a source by last use. A login with
/// the same username for the same site in several sources shows once, from
/// the first source.
pub fn merge(found: Vec<(SourceId, Vec<ItemSummary>)>, preferred: SourceId) -> Vec<Merged> {
    let rank = |s: SourceId| if s == preferred { 0 } else { 1 + s as u8 };
    let mut found = found;
    found.sort_by_key(|(s, _)| rank(*s));
    let mut out: Vec<Merged> = Vec::new();
    for (source, mut items) in found {
        items.sort_by(|a, b| {
            b.last_used
                .cmp(&a.last_used)
                .then_with(|| a.title.cmp(&b.title))
        });
        for item in items {
            let twin = (item.kind == ItemKind::Login && !item.username.is_empty())
                .then(|| {
                    out.iter_mut().find(|m| {
                        m.summary.kind == ItemKind::Login
                            && m.summary.username.eq_ignore_ascii_case(&item.username)
                            && m.summary.sites.iter().any(|s| item.sites.contains(s))
                    })
                })
                .flatten();
            match twin {
                Some(m) => {
                    if !m.sources.contains(&source) {
                        m.sources.push(source);
                    }
                }
                None => out.push(Merged {
                    source,
                    sources: vec![source],
                    summary: item,
                }),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(id: &str, user: &str, used: i64) -> ItemSummary {
        ItemSummary {
            id: id.into(),
            kind: ItemKind::Login,
            title: "example.com".into(),
            username: user.into(),
            sites: vec!["example.com".into()],
            last_used: used,
        }
    }

    #[test]
    fn duplicates_show_once_and_the_preferred_source_fills() {
        let found = vec![
            (
                SourceId::Bitwarden,
                vec![login("b1", "me", 5), login("b2", "work", 1)],
            ),
            (
                SourceId::Mado,
                vec![login("m1", "ME", 1), login("m2", "old", 9)],
            ),
            (SourceId::OnePassword, vec![login("o1", "me", 3)]),
        ];
        let merged = merge(found.clone(), SourceId::Mado);
        let ids: Vec<&str> = merged.iter().map(|m| m.summary.id.as_str()).collect();
        assert_eq!(ids, ["m2", "m1", "b2"]);
        assert_eq!(
            merged[1].sources,
            [SourceId::Mado, SourceId::OnePassword, SourceId::Bitwarden]
        );
        let merged = merge(found, SourceId::Bitwarden);
        assert_eq!(merged[0].summary.id, "b1");
        assert_eq!(
            merged[0].sources,
            [SourceId::Bitwarden, SourceId::Mado, SourceId::OnePassword]
        );
    }

    #[test]
    fn the_mado_source_finds_reveals_saves_and_updates() {
        let dir = tempfile::tempdir().expect("dir");
        let db = dir.path().join("vault.sqlite");
        mado_storage::open(
            &db,
            mado_storage::Kind::Vault,
            &dir.path().join("B"),
            &dir.path().join("X"),
        )
        .expect("db");
        let c = rusqlite::Connection::open(&db).expect("conn");
        let enclave = mado_vault::wrap::SoftEnclave::default();
        let (keys, _) = Vault::setup(&c, &enclave, dir.path(), 1).expect("setup");
        let src = MadoSource {
            db,
            keys: Arc::new(keys),
            now: 10,
        };
        let id = src
            .save(&NewLogin {
                url: "https://www.example.com/".into(),
                username: "me".into(),
                password: "pw".into(),
            })
            .expect("saved");
        let found = src.find("example.com", &[ItemKind::Login]).expect("find");
        assert_eq!(
            (
                found.len(),
                found[0].username.as_str(),
                found[0].sites[0].as_str()
            ),
            (1, "me", "example.com")
        );
        assert!(
            src.find("example.com", &[ItemKind::Card])
                .expect("find")
                .is_empty()
        );
        src.update_password(&id, "pw2").expect("updated");
        let Item::Login(l) = src.reveal(&id).expect("reveal") else {
            panic!("a login")
        };
        assert_eq!(l.password, "pw2");
        assert_eq!(src.totp(&id, 59).expect("totp"), None);
    }
}
