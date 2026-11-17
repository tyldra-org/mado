//! Bitwarden through its `bw` CLI (spec 11 §6.3). Mado unlocks it with the
//! master password the person types in Mado's sheet (in that child's
//! environment only) and keeps the session key in memory; each call runs
//! `bw` with `BW_SESSION` in its environment and a 15-second limit (30
//! for unlock and sync).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use mado_vault::item::{Card, Identity, Item, ItemKind, Login, site_of};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::runner::Runner;
use crate::source::{FillSource, ItemSummary, NewLogin, SourceError, SourceId, SourceStatus};

const LIMIT: Duration = Duration::from_secs(15);
const SLOW: Duration = Duration::from_secs(30);
const CACHE: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct Bitwarden {
    pub binary: Option<PathBuf>,
    pub runner: Arc<dyn Runner>,
    session: Mutex<Option<Zeroizing<String>>>,
    cache: Mutex<Option<(Instant, Vec<Value>)>>,
}

fn b64(data: &[u8]) -> String {
    data_encoding::BASE64.encode(data)
}

impl Bitwarden {
    pub fn new(binary: Option<PathBuf>, runner: Arc<dyn Runner>) -> Self {
        Self {
            binary,
            runner,
            session: Mutex::new(None),
            cache: Mutex::new(None),
        }
    }

    /// The session key, to keep wrapped by Mado's vault.
    pub fn session(&self) -> Option<Zeroizing<String>> {
        self.session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn set_session(&self, session: Option<Zeroizing<String>>) {
        *self.session.lock().unwrap_or_else(PoisonError::into_inner) = session;
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn bw(&self, args: &[&str], limit: Duration) -> Result<String, SourceError> {
        let bin = self
            .binary
            .as_deref()
            .ok_or(SourceError::Failed("bw is not installed".into()))?;
        let session = self.session();
        let env: Vec<(&str, &str)> = session
            .as_deref()
            .map(|s| ("BW_SESSION", s.as_str()))
            .into_iter()
            .collect();
        let out = self
            .runner
            .run(bin, args, &env, None, limit)
            .ok_or(SourceError::Failed("bw did not answer".into()))?;
        if out.ok {
            return Ok(out.stdout);
        }
        let err = out.stderr.to_lowercase();
        if err.contains("vault is locked") || err.contains("not logged in") {
            return Err(SourceError::Locked);
        }
        if err.contains("not found") {
            return Err(SourceError::NotFound);
        }
        Err(SourceError::Failed(
            out.stderr.trim().chars().take(300).collect(),
        ))
    }

    /// Unlocks with the master password and keeps the session; then syncs.
    pub fn unlock(&self, password: &str) -> Result<Zeroizing<String>, SourceError> {
        let bin = self
            .binary
            .as_deref()
            .ok_or(SourceError::Failed("bw is not installed".into()))?;
        let out = self
            .runner
            .run(
                bin,
                &["unlock", "--raw", "--passwordenv", "MADO_BW_PW"],
                &[("MADO_BW_PW", password)],
                None,
                SLOW,
            )
            .ok_or(SourceError::Failed("bw did not answer".into()))?;
        let session = Zeroizing::new(out.stdout.trim().to_owned());
        if !out.ok || session.is_empty() {
            let err = out.stderr.to_lowercase();
            return Err(if err.contains("invalid master password") {
                SourceError::Failed("bitwarden.wrongPassword".into())
            } else if err.contains("not logged in") {
                SourceError::Failed("bitwarden.notLoggedIn".into())
            } else {
                SourceError::Failed(out.stderr.trim().chars().take(300).collect())
            });
        }
        self.set_session(Some(session.clone()));
        let _ = self.bw(&["sync"], SLOW);
        Ok(session)
    }

    pub fn lock(&self) {
        self.set_session(None);
    }

    fn items(&self) -> Result<Vec<Value>, SourceError> {
        if self.session().is_none() {
            return Err(SourceError::Locked);
        }
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((at, items)) = &*cache
            && at.elapsed() < CACHE
        {
            return Ok(items.clone());
        }
        let text = self.bw(&["list", "items"], SLOW)?;
        let items: Vec<Value> =
            serde_json::from_str(&text).map_err(|e| SourceError::Failed(e.to_string()))?;
        *cache = Some((Instant::now(), items.clone()));
        Ok(items)
    }
}

fn kind_of(t: i64) -> Option<ItemKind> {
    Some(match t {
        1 => ItemKind::Login,
        3 => ItemKind::Card,
        4 => ItemKind::Identity,
        _ => return None,
    })
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

fn o(v: &Value) -> Option<String> {
    v.as_str().filter(|x| !x.is_empty()).map(str::to_owned)
}

/// A `bw list items` entry as the menu shows it.
pub fn summary(v: &Value) -> Option<ItemSummary> {
    let kind = kind_of(v["type"].as_i64()?)?;
    let username = match kind {
        ItemKind::Login => s(&v["login"]["username"]),
        ItemKind::Card => {
            let n = s(&v["card"]["number"]);
            let last: String = n
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("•••• {last}")
        }
        _ => s(&v["identity"]["email"]),
    };
    Some(ItemSummary {
        id: v["id"].as_str()?.to_owned(),
        kind,
        title: s(&v["name"]),
        username,
        sites: v["login"]["uris"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|u| u["uri"].as_str())
            .filter_map(site_of)
            .collect(),
        last_used: 0,
    })
}

/// A `bw get item` object as Mado's item JSON.
pub fn item(v: &Value) -> Result<Item, SourceError> {
    match v["type"].as_i64() {
        Some(1) => Ok(Item::Login(Login {
            title: s(&v["name"]),
            urls: v["login"]["uris"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|u| u["uri"].as_str().map(str::to_owned))
                .collect(),
            username: s(&v["login"]["username"]),
            password: s(&v["login"]["password"]),
            totp: o(&v["login"]["totp"]),
            ..Login::default()
        })),
        Some(3) => Ok(Item::Card(Card {
            name_on_card: s(&v["card"]["cardholderName"]),
            number: s(&v["card"]["number"]),
            exp_month: s(&v["card"]["expMonth"]).parse().unwrap_or(0),
            exp_year: s(&v["card"]["expYear"]).parse().unwrap_or(0),
            cvc: o(&v["card"]["code"]),
            nickname: o(&v["name"]),
        })),
        Some(4) => Ok(Item::Identity(Identity {
            given_name: s(&v["identity"]["firstName"]),
            family_name: s(&v["identity"]["lastName"]),
            email: o(&v["identity"]["email"]),
            phone: o(&v["identity"]["phone"]),
            company: o(&v["identity"]["company"]),
            ..Identity::default()
        })),
        _ => Err(SourceError::NotFound),
    }
}

impl FillSource for Bitwarden {
    fn id(&self) -> SourceId {
        SourceId::Bitwarden
    }

    fn status(&self) -> SourceStatus {
        if self.binary.is_none() {
            return SourceStatus::NotInstalled;
        }
        let Ok(text) = self.bw(&["status"], LIMIT) else {
            return SourceStatus::Error("bitwarden.noAnswer".into());
        };
        let v: Value = serde_json::from_str(&text).unwrap_or_default();
        match v["status"].as_str() {
            Some("unlocked") if self.session().is_some() => SourceStatus::Ready,
            Some("unauthenticated") => SourceStatus::Error("bitwarden.notLoggedIn".into()),
            _ => SourceStatus::Locked,
        }
    }

    fn find(&self, site: &str, kinds: &[ItemKind]) -> Result<Vec<ItemSummary>, SourceError> {
        Ok(self
            .items()?
            .iter()
            .filter_map(summary)
            .filter(|i| kinds.contains(&i.kind))
            .filter(|i| i.kind != ItemKind::Login || i.sites.iter().any(|x| x == site))
            .collect())
    }

    fn reveal(&self, id: &str) -> Result<Item, SourceError> {
        if let Some(v) = self.items()?.into_iter().find(|v| v["id"] == id) {
            return item(&v);
        }
        let v: Value = serde_json::from_str(&self.bw(&["get", "item", id], LIMIT)?)
            .map_err(|e| SourceError::Failed(e.to_string()))?;
        item(&v)
    }

    fn totp(&self, id: &str, _now_secs: u64) -> Result<Option<String>, SourceError> {
        match self.bw(&["get", "totp", id], LIMIT) {
            Ok(code) => Ok(Some(code.trim().to_owned()).filter(|c| !c.is_empty())),
            Err(SourceError::Failed(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn save(&self, login: &NewLogin) -> Result<String, SourceError> {
        let name = site_of(&login.url).unwrap_or_else(|| login.url.clone());
        let body = json!({
            "type": 1, "name": name, "notes": null, "favorite": false,
            "login": { "username": login.username, "password": login.password, "uris": [{ "uri": login.url, "match": null }] },
        });
        let encoded = b64(body.to_string().as_bytes());
        let out = self.bw(&["create", "item", &encoded], LIMIT)?;
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let v: Value =
            serde_json::from_str(&out).map_err(|e| SourceError::Failed(e.to_string()))?;
        v["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or(SourceError::Failed("bw gave no ID".into()))
    }

    fn update_password(&self, id: &str, password: &str) -> Result<(), SourceError> {
        let mut v: Value = serde_json::from_str(&self.bw(&["get", "item", id], LIMIT)?)
            .map_err(|e| SourceError::Failed(e.to_string()))?;
        v["login"]["password"] = json!(password);
        let encoded = b64(v.to_string().as_bytes());
        self.bw(&["edit", "item", id, &encoded], LIMIT)?;
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::Output;
    use std::path::Path;

    /// One call: its arguments and environment.
    type Call = (Vec<String>, Vec<(String, String)>);

    #[derive(Debug, Default)]
    struct FakeBw {
        calls: Mutex<Vec<Call>>,
    }

    impl Runner for FakeBw {
        fn run(
            &self,
            _: &Path,
            args: &[&str],
            env: &[(&str, &str)],
            _: Option<&str>,
            _: Duration,
        ) -> Option<Output> {
            self.calls.lock().expect("lock").push((
                args.iter().map(|s| (*s).to_owned()).collect(),
                env.iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            ));
            let session = env
                .iter()
                .any(|(k, v)| *k == "BW_SESSION" && *v == "S3SSION");
            let ok = |s: &str| {
                Some(Output {
                    ok: true,
                    stdout: s.into(),
                    stderr: String::new(),
                })
            };
            let locked = Some(Output {
                ok: false,
                stdout: String::new(),
                stderr: "Vault is locked.".into(),
            });
            match args[0] {
                "unlock" if env.contains(&("MADO_BW_PW", "right")) => ok("S3SSION"),
                "unlock" => Some(Output {
                    ok: false,
                    stdout: String::new(),
                    stderr: "Invalid master password.".into(),
                }),
                "status" if session => ok(r#"{"status":"unlocked"}"#),
                "status" => ok(r#"{"status":"locked"}"#),
                _ if !session => locked,
                "sync" => ok("Syncing complete."),
                "list" => ok(
                    r#"[{"id":"b1","type":1,"name":"Example","login":{"username":"me","password":"bw-pw","totp":null,"uris":[{"uri":"https://www.example.com/"}]}},
                    {"id":"b2","type":3,"name":"Card","card":{"cardholderName":"Me","number":"4242424242424242","expMonth":"7","expYear":"2030","code":"999"}},
                    {"id":"b3","type":2,"name":"Note"}]"#,
                ),
                "get" if args[1] == "totp" => ok("246810"),
                "get" => ok(
                    r#"{"id":"b1","type":1,"name":"Example","login":{"username":"me","password":"bw-pw","uris":[{"uri":"https://www.example.com/"}]}}"#,
                ),
                "create" => ok(r#"{"id":"b9"}"#),
                "edit" => ok("{}"),
                _ => None,
            }
        }
    }

    fn source() -> (Bitwarden, Arc<FakeBw>) {
        let fake = Arc::new(FakeBw::default());
        (
            Bitwarden::new(
                Some("/opt/homebrew/bin/bw".into()),
                Arc::clone(&fake) as Arc<dyn Runner>,
            ),
            fake,
        )
    }

    #[test]
    fn unlocking_keeps_the_session_and_items_follow() {
        let (bw, fake) = source();
        assert_eq!(bw.status(), SourceStatus::Locked);
        assert!(matches!(
            bw.find("example.com", &[ItemKind::Login]),
            Err(SourceError::Locked)
        ));
        assert!(
            matches!(bw.unlock("wrong"), Err(SourceError::Failed(m)) if m == "bitwarden.wrongPassword")
        );
        assert_eq!(bw.unlock("right").expect("unlocked").as_str(), "S3SSION");
        assert_eq!(bw.status(), SourceStatus::Ready);
        let found = bw.find("example.com", &[ItemKind::Login]).expect("find");
        assert_eq!((found.len(), found[0].username.as_str()), (1, "me"));
        let Item::Card(c) = bw.reveal("b2").expect("card") else {
            panic!("a card")
        };
        assert_eq!(
            (c.exp_month, c.exp_year, c.cvc.as_deref()),
            (7, 2030, Some("999"))
        );
        assert_eq!(bw.totp("b1", 0).expect("totp").as_deref(), Some("246810"));
        let calls = fake.calls.lock().expect("lock").clone();
        let unlock = calls
            .iter()
            .find(|(a, e)| a[0] == "unlock" && e.iter().any(|(_, v)| v == "right"))
            .expect("unlock");
        assert!(
            !unlock.0.iter().any(|a| a.contains("right")),
            "the password never goes in argv"
        );
        assert!(
            calls.iter().any(|(a, _)| a[0] == "sync"),
            "synced after unlocking"
        );
        bw.lock();
        assert!(matches!(
            bw.find("example.com", &[ItemKind::Login]),
            Err(SourceError::Locked)
        ));
    }

    #[test]
    fn saving_and_updating_send_encoded_items() {
        let (bw, fake) = source();
        bw.unlock("right").expect("unlocked");
        assert_eq!(
            bw.save(&NewLogin {
                url: "https://shop.example.org/".into(),
                username: "u".into(),
                password: "p".into()
            })
            .expect("saved"),
            "b9"
        );
        bw.update_password("b1", "new").expect("updated");
        let calls = fake.calls.lock().expect("lock").clone();
        let decode = |s: &str| -> Value {
            serde_json::from_slice(&data_encoding::BASE64.decode(s.as_bytes()).expect("base64"))
                .expect("json")
        };
        let create = calls
            .iter()
            .find(|(a, _)| a[0] == "create")
            .expect("create");
        let body = decode(&create.0[2]);
        assert_eq!(
            (body["name"].as_str(), body["login"]["password"].as_str()),
            (Some("example.org"), Some("p"))
        );
        let edit = calls.iter().find(|(a, _)| a[0] == "edit").expect("edit");
        assert_eq!(decode(&edit.0[3])["login"]["password"], "new");
    }
}
