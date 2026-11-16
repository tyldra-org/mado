//! 1Password through its `op` CLI (spec 11 §6.2). Needs the desktop app's
//! "Integrate with 1Password CLI"; every call has a 15-second limit and
//! passes values as separate arguments.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use mado_vault::item::{Card, Identity, Item, ItemKind, Login, site_of};
use serde_json::Value;

use crate::runner::Runner;
use crate::source::{FillSource, ItemSummary, NewLogin, SourceError, SourceId, SourceStatus};

const LIMIT: Duration = Duration::from_secs(15);
const CACHE: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct OnePassword {
    pub binary: Option<PathBuf>,
    pub runner: Arc<dyn Runner>,
    cache: Mutex<Option<(Instant, Vec<ItemSummary>)>>,
}

impl OnePassword {
    pub fn new(binary: Option<PathBuf>, runner: Arc<dyn Runner>) -> Self {
        Self {
            binary,
            runner,
            cache: Mutex::new(None),
        }
    }

    fn op(&self, args: &[&str]) -> Result<String, SourceError> {
        let bin = self
            .binary
            .as_deref()
            .ok_or(SourceError::Failed("op is not installed".into()))?;
        let out = self
            .runner
            .run(bin, args, &[], None, LIMIT)
            .ok_or(SourceError::Failed("op did not answer".into()))?;
        if out.ok {
            return Ok(out.stdout);
        }
        let err = out.stderr.to_lowercase();
        if err.contains("not signed in")
            || err.contains("session expired")
            || err.contains("authorization")
        {
            return Err(SourceError::Locked);
        }
        if err.contains("isn't an item") || err.contains("not found") {
            return Err(SourceError::NotFound);
        }
        Err(SourceError::Failed(
            out.stderr.trim().chars().take(300).collect(),
        ))
    }

    fn list(&self) -> Result<Vec<ItemSummary>, SourceError> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((at, items)) = &*cache
            && at.elapsed() < CACHE
        {
            return Ok(items.clone());
        }
        let text = self.op(&[
            "item",
            "list",
            "--categories",
            "Login,Credit Card,Identity",
            "--format",
            "json",
        ])?;
        let items = parse_list(&text)?;
        *cache = Some((Instant::now(), items.clone()));
        Ok(items)
    }

    fn forget_cache(&self) {
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

fn kind_of(category: &str) -> Option<ItemKind> {
    Some(match category {
        "LOGIN" => ItemKind::Login,
        "CREDIT_CARD" => ItemKind::Card,
        "IDENTITY" => ItemKind::Identity,
        _ => return None,
    })
}

fn time_of(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| s.parse::<jiff_like::Stamp>().ok())
        .map_or(0, |s| s.0)
}

/// RFC 3339 times, to milliseconds, without another dependency.
mod jiff_like {
    pub struct Stamp(pub i64);
    impl std::str::FromStr for Stamp {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, ()> {
            let n = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<i64>().ok()).ok_or(());
            let (y, mo, d, h, mi, se) = (
                n(0, 4)?,
                n(5, 7)?,
                n(8, 10)?,
                n(11, 13)?,
                n(14, 16)?,
                n(17, 19)?,
            );
            // Days from 1970 by the civil calendar.
            let (yy, mm) = if mo <= 2 {
                (y - 1, mo + 9)
            } else {
                (y, mo - 3)
            };
            let era = yy.div_euclid(400);
            let yoe = yy - era * 400;
            let doy = (153 * mm + 2) / 5 + d - 1;
            let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
            let days = era * 146_097 + doe - 719_468;
            Ok(Self(((days * 24 + h) * 60 + mi) * 60_000 + se * 1000))
        }
    }
}

/// `op item list --format json`.
pub fn parse_list(text: &str) -> Result<Vec<ItemSummary>, SourceError> {
    let list: Vec<Value> =
        serde_json::from_str(text).map_err(|e| SourceError::Failed(e.to_string()))?;
    Ok(list
        .iter()
        .filter_map(|v| {
            let kind = kind_of(v["category"].as_str()?)?;
            Some(ItemSummary {
                id: v["id"].as_str()?.to_owned(),
                kind,
                title: v["title"].as_str().unwrap_or_default().to_owned(),
                username: v["additional_information"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                sites: v["urls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|u| u["href"].as_str())
                    .filter_map(site_of)
                    .collect(),
                last_used: time_of(&v["updated_at"]),
            })
        })
        .collect())
}

fn field<'a>(item: &'a Value, purpose_or_id: &str) -> Option<&'a str> {
    item["fields"].as_array()?.iter().find_map(|f| {
        (f["purpose"].as_str() == Some(purpose_or_id) || f["id"].as_str() == Some(purpose_or_id))
            .then(|| f["value"].as_str())
            .flatten()
    })
}

/// `op item get <id> --format json --reveal`.
pub fn parse_item(text: &str) -> Result<Item, SourceError> {
    let v: Value = serde_json::from_str(text).map_err(|e| SourceError::Failed(e.to_string()))?;
    let title = v["title"].as_str().unwrap_or_default().to_owned();
    match v["category"].as_str() {
        Some("LOGIN") => Ok(Item::Login(Login {
            title,
            urls: v["urls"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|u| u["href"].as_str().map(str::to_owned))
                .collect(),
            username: field(&v, "USERNAME").unwrap_or_default().to_owned(),
            password: field(&v, "PASSWORD").unwrap_or_default().to_owned(),
            ..Login::default()
        })),
        Some("CREDIT_CARD") => {
            // `expiry` is YYYYMM.
            let exp = field(&v, "expiry").unwrap_or_default();
            let (y, m) = exp.split_at(exp.len().min(4));
            Ok(Item::Card(Card {
                name_on_card: field(&v, "cardholder").unwrap_or_default().to_owned(),
                number: field(&v, "ccnum").unwrap_or_default().to_owned(),
                exp_month: m.parse().unwrap_or(0),
                exp_year: y.parse().unwrap_or(0),
                cvc: field(&v, "cvv").map(str::to_owned),
                nickname: Some(title),
            }))
        }
        Some("IDENTITY") => Ok(Item::Identity(Identity {
            given_name: field(&v, "firstname").unwrap_or_default().to_owned(),
            family_name: field(&v, "lastname").unwrap_or_default().to_owned(),
            email: field(&v, "email").map(str::to_owned),
            phone: field(&v, "defphone").map(str::to_owned),
            company: field(&v, "company").map(str::to_owned),
            ..Identity::default()
        })),
        _ => Err(SourceError::NotFound),
    }
}

impl FillSource for OnePassword {
    fn id(&self) -> SourceId {
        SourceId::OnePassword
    }

    fn status(&self) -> SourceStatus {
        if self.binary.is_none() {
            return SourceStatus::NotInstalled;
        }
        match self.op(&["whoami", "--format", "json"]) {
            Ok(_) => SourceStatus::Ready,
            Err(SourceError::Locked) => SourceStatus::Locked,
            Err(SourceError::Failed(m)) if m.to_lowercase().contains("no account") => {
                SourceStatus::Error("onePassword.integrationOff".into())
            }
            Err(e) => SourceStatus::Error(e.to_string()),
        }
    }

    fn find(&self, site: &str, kinds: &[ItemKind]) -> Result<Vec<ItemSummary>, SourceError> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|i| kinds.contains(&i.kind))
            .filter(|i| i.kind != ItemKind::Login || i.sites.iter().any(|s| s == site))
            .collect())
    }

    fn reveal(&self, id: &str) -> Result<Item, SourceError> {
        parse_item(&self.op(&["item", "get", id, "--format", "json", "--reveal"])?)
    }

    fn totp(&self, id: &str, _now_secs: u64) -> Result<Option<String>, SourceError> {
        match self.op(&["item", "get", id, "--otp"]) {
            Ok(code) => Ok(Some(code.trim().to_owned()).filter(|c| !c.is_empty())),
            Err(SourceError::Failed(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn save(&self, login: &NewLogin) -> Result<String, SourceError> {
        let title = site_of(&login.url).unwrap_or_else(|| login.url.clone());
        let user = format!("username={}", login.username);
        let pass = format!("password={}", login.password);
        let out = self.op(&[
            "item",
            "create",
            "--category",
            "login",
            "--title",
            &title,
            "--url",
            &login.url,
            "--format",
            "json",
            &user,
            &pass,
        ])?;
        self.forget_cache();
        let v: Value =
            serde_json::from_str(&out).map_err(|e| SourceError::Failed(e.to_string()))?;
        v["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or(SourceError::Failed("op gave no ID".into()))
    }

    fn update_password(&self, id: &str, password: &str) -> Result<(), SourceError> {
        let pass = format!("password={password}");
        self.op(&["item", "edit", id, &pass])?;
        self.forget_cache();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::Output;
    use std::path::Path;

    /// Answers by the first two arguments; records every call.
    #[derive(Debug, Default)]
    struct FakeOp {
        calls: Mutex<Vec<Vec<String>>>,
        signed_in: bool,
    }

    impl Runner for FakeOp {
        fn run(
            &self,
            _: &Path,
            args: &[&str],
            _: &[(&str, &str)],
            _: Option<&str>,
            _: Duration,
        ) -> Option<Output> {
            self.calls
                .lock()
                .expect("lock")
                .push(args.iter().map(|s| (*s).to_owned()).collect());
            let ok = |s: &str| {
                Some(Output {
                    ok: true,
                    stdout: s.into(),
                    stderr: String::new(),
                })
            };
            if !self.signed_in {
                return Some(Output {
                    ok: false,
                    stdout: String::new(),
                    stderr: "[ERROR] You are not signed in".into(),
                });
            }
            match (args[0], args.get(1).copied().unwrap_or("")) {
                ("whoami", _) => ok(r#"{"email":"me@example.com"}"#),
                ("item", "list") => ok(r#"[
                    {"id":"l1","title":"Example","category":"LOGIN","additional_information":"me","urls":[{"href":"https://www.example.com/login","primary":true}],"updated_at":"2026-11-01T10:00:00Z"},
                    {"id":"l2","title":"Other","category":"LOGIN","additional_information":"x","urls":[{"href":"https://other.org"}],"updated_at":"2026-10-01T10:00:00Z"},
                    {"id":"c1","title":"Visa","category":"CREDIT_CARD","additional_information":"•••• 1111","updated_at":"2026-10-01T10:00:00Z"},
                    {"id":"n1","title":"Note","category":"SECURE_NOTE"}
                ]"#),
                ("item", "get") if args.contains(&"--otp") => ok("123456\n"),
                ("item", "get") if args[2] == "c1" => ok(
                    r#"{"id":"c1","title":"Visa","category":"CREDIT_CARD","fields":[
                    {"id":"cardholder","value":"Me"},{"id":"ccnum","value":"4111111111111111"},{"id":"expiry","value":"203104"},{"id":"cvv","value":"123"}]}"#,
                ),
                ("item", "get") => ok(
                    r#"{"id":"l1","title":"Example","category":"LOGIN","urls":[{"href":"https://www.example.com/login"}],"fields":[
                    {"id":"username","purpose":"USERNAME","value":"me"},{"id":"password","purpose":"PASSWORD","value":"hunter2"}]}"#,
                ),
                ("item", "create") => ok(r#"{"id":"new1"}"#),
                ("item", "edit") => ok("{}"),
                _ => None,
            }
        }
    }

    fn source(signed_in: bool) -> (OnePassword, Arc<FakeOp>) {
        let fake = Arc::new(FakeOp {
            signed_in,
            ..FakeOp::default()
        });
        (
            OnePassword::new(
                Some("/opt/homebrew/bin/op".into()),
                Arc::clone(&fake) as Arc<dyn Runner>,
            ),
            fake,
        )
    }

    #[test]
    fn items_are_found_by_site_revealed_and_cached() {
        let (op, fake) = source(true);
        assert_eq!(op.status(), SourceStatus::Ready);
        let found = op.find("example.com", &[ItemKind::Login]).expect("find");
        assert_eq!(
            found.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["l1"]
        );
        assert_eq!(found[0].last_used, 1_793_527_200_000);
        let cards = op.find("anything.com", &[ItemKind::Card]).expect("cards");
        assert_eq!(cards[0].id, "c1");
        let lists = fake
            .calls
            .lock()
            .expect("lock")
            .iter()
            .filter(|c| c[1] == "list")
            .count();
        assert_eq!(lists, 1, "the list is cached");
        let Item::Login(l) = op.reveal("l1").expect("reveal") else {
            panic!("a login")
        };
        assert_eq!(
            (l.username.as_str(), l.password.as_str()),
            ("me", "hunter2")
        );
        let Item::Card(c) = op.reveal("c1").expect("card") else {
            panic!("a card")
        };
        assert_eq!(
            (c.exp_month, c.exp_year, c.cvc.as_deref()),
            (4, 2031, Some("123"))
        );
        assert_eq!(op.totp("l1", 0).expect("totp").as_deref(), Some("123456"));
    }

    #[test]
    fn saving_passes_values_as_separate_arguments() {
        let (op, fake) = source(true);
        let id = op
            .save(&NewLogin {
                url: "https://www.example.com/".into(),
                username: "me; rm -rf /".into(),
                password: "p w".into(),
            })
            .expect("saved");
        assert_eq!(id, "new1");
        op.update_password("l1", "new pass").expect("updated");
        let calls = fake.calls.lock().expect("lock").clone();
        let create = calls.iter().find(|c| c[1] == "create").expect("create");
        assert!(create.contains(&"username=me; rm -rf /".to_owned()));
        assert!(create.contains(&"password=p w".to_owned()));
        assert!(create.contains(&"example.com".to_owned()), "titled by site");
        assert!(
            calls
                .iter()
                .any(|c| c == &["item", "edit", "l1", "password=new pass"])
        );
    }

    #[test]
    fn signed_out_is_locked_and_missing_is_not_installed() {
        let (op, _) = source(false);
        assert_eq!(op.status(), SourceStatus::Locked);
        assert!(matches!(
            op.find("example.com", &[ItemKind::Login]),
            Err(SourceError::Locked)
        ));
        let none = OnePassword::new(None, Arc::new(FakeOp::default()));
        assert_eq!(none.status(), SourceStatus::NotInstalled);
    }
}
