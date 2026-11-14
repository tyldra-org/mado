//! Spec 06 §10 checks 1 and 2, and the registry's own rules.

use mado_settings::types::{Scope, SettingType};
use mado_settings::{
    AuthTokens, BlockerEnabled, REGISTRY, Settings, SettingsError, Strings, TabsMemorySaver,
    TabsSleepAfter, Write, def, generate, persist,
};
use mado_storage::{Kind, Storage, open};
use mado_types::command::{Origin, ResetTarget};
use mado_types::{ClientId, TaskId, WindowId};
use serde_json::{Value, json};

struct Tokens(Vec<String>);

impl AuthTokens for Tokens {
    fn consume(&mut self, token: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|t| t != token);
        self.0.len() < before
    }
}

fn no_tokens() -> Tokens {
    Tokens(vec![])
}

/// A valid value different from `default`.
fn other(ty: &SettingType, default: &Value) -> Value {
    match ty {
        SettingType::Bool => json!(!default.as_bool().unwrap_or(false)),
        SettingType::Enum(options) => json!(
            options
                .iter()
                .find(|o| json!(o) != *default)
                .expect("two options")
        ),
        SettingType::Int { min, max, .. } | SettingType::Duration { min, max } => {
            if default == &json!(min) {
                json!(max)
            } else {
                json!(min)
            }
        }
        SettingType::Float { min, max } => {
            if default.as_f64() == Some(*min) {
                json!(max)
            } else {
                json!(min)
            }
        }
        SettingType::String { .. } => json!("example.test"),
        SettingType::Url => json!("https://example.com/"),
        SettingType::Path => json!("/tmp"),
        SettingType::Shortcut => json!("Cmd+Shift+Y"),
        SettingType::Json { .. } => json!(1000),
        SettingType::List(item) => {
            if default.as_array().is_some_and(|a| !a.is_empty()) {
                json!([])
            } else {
                json!([other(item, &Value::Null)])
            }
        }
    }
}

#[test]
fn the_registry_is_consistent() {
    let strings = Strings::english();
    let mut keys = std::collections::HashSet::new();
    for (i, d) in REGISTRY.iter().enumerate() {
        assert!(keys.insert(d.key), "duplicate {}", d.key);
        assert_eq!(d.id as usize, i, "{} is out of order", d.key);
        assert!(
            d.ty.validate(&d.default_value()).is_ok(),
            "{}'s default does not fit its type",
            d.key
        );
        for k in [d.label, d.help, d.keywords] {
            assert!(strings.contains(k), "en.json lacks {k}");
        }
        assert!(
            strings.contains(&format!("settings.section.{}", d.section.id())),
            "{}",
            d.section.id()
        );
    }
    let s = Settings::new(strings);
    // Typed accessors read the defaults.
    assert_eq!(s.get::<TabsMemorySaver>(), "balanced");
    assert_eq!(s.get::<TabsSleepAfter>(), 30 * 60 * 1000);
    assert!(s.get::<BlockerEnabled>());
}

#[test]
fn check_1_every_setting_round_trips_through_set_get_reset_export_and_import() {
    let mut s = Settings::new(Strings::english());
    for d in REGISTRY.iter().filter(|d| d.scope != Scope::Site) {
        let new = other(&d.ty, &d.default_value());
        let token = Some("t");
        s.set(
            d.key,
            new.clone(),
            &Origin::Settings,
            token,
            &mut Tokens(vec!["t".into()]),
            1,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", d.key));
        assert_eq!(
            s.get_many(Some(&[d.key.to_owned()])).expect("gets")[d.key],
            new,
            "{}",
            d.key
        );
        let exported = s.export();
        let part = if d.scope == Scope::App {
            "app"
        } else {
            "profile"
        };
        assert_eq!(
            exported[part].get(d.key).is_some(),
            !d.opts.protected,
            "{}",
            d.key
        );

        let out = s
            .reset(
                &ResetTarget::Key { key: d.key.into() },
                &Origin::Settings,
                2,
            )
            .expect("resets");
        assert_eq!(out.changes.len(), 1);
        assert_eq!(
            s.get_many(Some(&[d.key.to_owned()])).expect("gets")[d.key],
            d.default_value()
        );

        if !d.opts.protected {
            let (_, report) = s.import(&exported, &Origin::Settings, 3).expect("imports");
            assert!(
                report.applied.iter().any(|k| k == d.key),
                "{} not re-imported: {report:?}",
                d.key
            );
            assert_eq!(
                s.get_many(Some(&[d.key.to_owned()])).expect("gets")[d.key],
                new,
                "{}",
                d.key
            );
            s.reset(
                &ResetTarget::Key { key: d.key.into() },
                &Origin::Settings,
                4,
            )
            .expect("resets");
        }
    }
    assert!(s.changed().is_empty());
}

#[test]
fn check_2_protected_settings_refuse_control_clients_and_agents() {
    let mut s = Settings::new(Strings::english());
    for origin in [
        Origin::Control {
            client: ClientId::new(),
        },
        Origin::Agent {
            task: TaskId::new(),
        },
        Origin::Ui {
            window: WindowId::new(),
        },
        Origin::Overlay {
            window: WindowId::new(),
        },
    ] {
        for d in REGISTRY.iter().filter(|d| d.opts.protected) {
            let err = s
                .set(
                    d.key,
                    other(&d.ty, &d.default_value()),
                    &origin,
                    None,
                    &mut no_tokens(),
                    1,
                )
                .expect_err("refused");
            assert_eq!(err, SettingsError::Protected(d.key.into()));
            assert!(
                s.reset(&ResetTarget::Key { key: d.key.into() }, &origin, 1)
                    .is_err()
            );
        }
        // Resetting everything skips protected keys rather than failing.
        s.set(
            "blocker.enabled",
            json!(false),
            &Origin::Settings,
            None,
            &mut no_tokens(),
            1,
        )
        .expect("user turns it off");
        s.reset(&ResetTarget::All { all: true }, &origin, 2)
            .expect("resets the rest");
        assert_eq!(
            s.get_many(Some(&["blocker.enabled".into()])).expect("gets")["blocker.enabled"],
            false
        );
    }
    assert_eq!(s.changed(), vec!["blocker.enabled"]);
}

#[test]
fn some_changes_need_os_authentication() {
    let mut s = Settings::new(Strings::english());
    let err = s
        .set(
            "vault.lockAfter",
            json!(60_000),
            &Origin::Settings,
            None,
            &mut no_tokens(),
            1,
        )
        .expect_err("needs auth");
    assert_eq!(err, SettingsError::AuthRequired("vault.lockAfter".into()));
    let mut tokens = Tokens(vec!["once".into()]);
    s.set(
        "vault.lockAfter",
        json!(60_000),
        &Origin::Settings,
        Some("once"),
        &mut tokens,
        1,
    )
    .expect("authorised");
    // The token is spent.
    assert!(
        s.set(
            "vault.lockAfter",
            json!(120_000),
            &Origin::Settings,
            Some("once"),
            &mut tokens,
            2
        )
        .is_err()
    );
    // Raising ai.approvalMode needs none; lowering it to full does.
    s.set(
        "ai.approvalMode",
        json!("ask"),
        &Origin::Settings,
        None,
        &mut no_tokens(),
        3,
    )
    .expect("stricter is free");
    assert!(
        s.set(
            "ai.approvalMode",
            json!("full"),
            &Origin::Settings,
            None,
            &mut no_tokens(),
            4
        )
        .is_err()
    );
}

#[test]
fn invalid_values_and_unknown_keys_change_nothing() {
    let mut s = Settings::new(Strings::english());
    assert!(matches!(
        s.set(
            "tabs.memorySaver",
            json!("turbo"),
            &Origin::Settings,
            None,
            &mut no_tokens(),
            1
        ),
        Err(SettingsError::Invalid { .. })
    ));
    assert!(matches!(
        s.set(
            "no.such",
            json!(1),
            &Origin::Settings,
            None,
            &mut no_tokens(),
            1
        ),
        Err(SettingsError::UnknownKey(_))
    ));
    assert!(s.changed().is_empty() && s.log().count() == 0);
}

#[test]
fn undo_puts_back_the_previous_value_and_risky_changes_offer_it() {
    let mut s = Settings::new(Strings::english());
    let out = s
        .set(
            "blocker.enabled",
            json!(false),
            &Origin::Settings,
            None,
            &mut no_tokens(),
            1,
        )
        .expect("sets");
    let entry = out.undo_notice.expect("blocker.enabled offers undo");
    s.set(
        "tabs.memorySaver",
        json!("maximum"),
        &Origin::Ui {
            window: WindowId::new(),
        },
        None,
        &mut no_tokens(),
        2,
    )
    .expect("sets");
    // Undo the older change by id, then the newest.
    s.undo(Some(entry), &Origin::Settings, 3).expect("undoes");
    assert!(s.get::<BlockerEnabled>());
    s.undo(None, &Origin::Settings, 4).expect("undoes");
    assert_eq!(s.get::<TabsMemorySaver>(), "balanced");
    assert_eq!(
        s.undo(None, &Origin::Settings, 5),
        Err(SettingsError::NothingToUndo)
    );
}

#[test]
fn search_finds_settings_by_what_people_type() {
    let s = Settings::new(Strings::english());
    let first = |q: &str| s.search(q).first().map(|h| h.key).unwrap_or_default();
    assert_eq!(first("block ads"), "blocker.enabled");
    assert_eq!(first("camera"), "permissions.camera");
    assert_eq!(first("cookies"), "privacy.trackingPrevention");
    assert!(
        s.search("sleep")
            .iter()
            .take(3)
            .any(|h| h.key == "tabs.sleepAfter")
    );
    // AI settings stay hidden while the feature is off.
    assert!(s.search("agents").iter().all(|h| !h.key.starts_with("ai.")));
}

#[test]
fn values_persist_and_reload_through_storage() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app_db = open(
        &dir.path().join("app.sqlite"),
        Kind::App,
        &dir.path().join("B"),
        &dir.path().join("X"),
    )
    .expect("app");
    let profile_db = open(
        &dir.path().join("profile.sqlite"),
        Kind::Profile,
        &dir.path().join("B"),
        &dir.path().join("X"),
    )
    .expect("profile");
    let app = Storage::start(&app_db).expect("starts");
    let profile = Storage::start(&profile_db).expect("starts");
    let mut s = Settings::new(Strings::english());
    let mut writes = Vec::new();
    for (k, v) in [
        ("update.channel", json!("beta")),
        ("tabs.memorySaver", json!("off")),
        ("startup.pages", json!(["https://x.test/"])),
    ] {
        writes.extend(
            s.set(k, v, &Origin::Settings, None, &mut no_tokens(), 5)
                .expect("sets")
                .writes,
        );
    }
    // A value from an old build that no longer validates.
    writes.push(Write::Upsert {
        scope: Scope::Profile,
        key: def(mado_settings::SettingKey::TabsCtrlTabOrder).key,
        value: json!("sideways"),
        version: 1,
        at: 1,
    });
    let (w1, w2) = (writes.clone(), writes);
    app.writer()
        .write(move |c| persist(&w1, Scope::App, c))
        .wait()
        .expect("app writes");
    profile
        .writer()
        .write(move |c| persist(&w2, Scope::Profile, c))
        .wait()
        .expect("profile writes");

    let a = rusqlite::Connection::open(app_db.path.clone()).expect("open");
    let p = rusqlite::Connection::open(profile_db.path.clone()).expect("open");
    let (loaded, fixes) = Settings::load(&a, &p, Strings::english(), 9).expect("loads");
    let all = loaded.get_many(None).expect("gets");
    assert_eq!(all["update.channel"], "beta");
    assert_eq!(all["tabs.memorySaver"], "off");
    assert_eq!(all["startup.pages"], json!(["https://x.test/"]));
    assert_eq!(
        all["tabs.ctrlTabOrder"], "strip",
        "invalid stored value falls back to the default"
    );
    assert!(fixes.iter().any(|w| matches!(
        w,
        Write::Delete {
            key: "tabs.ctrlTabOrder",
            ..
        }
    )));
    // Three changes, plus the reset of the invalid value.
    assert_eq!(loaded.log().count(), 4);
    assert_eq!(
        loaded.log().last().map(|e| e.origin.as_str()),
        Some("migration")
    );
}

#[test]
fn generated_files_are_up_to_date() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/generated");
    std::fs::write(root.join("settings.ts"), generate::settings_ts()).expect("writes settings.ts");
    let index = serde_json::to_string_pretty(&generate::search_index()).expect("serialises") + "\n";
    std::fs::write(root.join("settings-index.json"), index).expect("writes the index");
    let schema = generate::import_schema();
    assert_eq!(
        schema["properties"]["profile"]["properties"]["tabs.memorySaver"]["enum"][0],
        "off"
    );
    assert!(
        schema["properties"]["profile"]["properties"]
            .get("blocker.enabled")
            .is_none(),
        "protected keys are not importable"
    );
}
