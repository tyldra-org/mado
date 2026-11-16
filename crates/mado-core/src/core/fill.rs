//! Filling (spec 11 §4): the fill menu at a focused field, `vault.fill`,
//! two-step sign-in and the one-time code after a login.

use std::collections::BTreeMap;

use mado_engine::event::FrameInfo;
use mado_engine::page::{
    DetectedField, DetectedForm, FieldClass, FieldFocused, FillDone, FormKind,
};
use mado_engine::tab::World;
use mado_engine::{EngineOp, TabCommand};
use mado_types::command::Origin;
use mado_types::patch::PatchRecorder;
use mado_types::state::{FillMenuItem, FillMenuPrompt, PromptState, Rect, VaultStatus};
use mado_types::{ErrorCode, MadoError, PromptId, TabId};
use mado_vault::item::{Item, site_of};
use mado_vault::{Stored, Vault, VaultError};
use serde_json::{Value, json};

use super::vault::VaultOutcome;
use super::{Core, Reply};

/// A fill needs a native click or key press this recently (11 §4.1 step 3).
pub(super) const GESTURE_MS: i64 = 1_000;
/// Two-step sign-in and the code after a login wait this long (11 §4.2).
const FOLLOW_UP_MS: i64 = 60_000;

/// Where a fill menu's values would go.
#[derive(Debug, Clone)]
pub struct FillTarget {
    pub(super) tab: TabId,
    pub(super) frame: u64,
    pub(super) origin: String,
    pub(super) main: bool,
    pub(super) form_key: String,
    pub(super) kind: FormKind,
    /// The page's site: the frame's, or the tab's for a payment frame.
    pub(super) site: String,
    /// The focused field's `passwordrules`, for the generator.
    pub(super) rules: Option<String>,
}

/// What the core remembers between fills on one tab.
#[derive(Debug, Clone)]
pub(super) struct FollowUp {
    pub site: String,
    pub item: String,
    pub at: i64,
}

fn kind_name(k: FormKind) -> &'static str {
    match k {
        FormKind::Login => "login",
        FormKind::UsernameOnly => "usernameOnly",
        FormKind::Signup => "signup",
        FormKind::ChangePassword => "changePassword",
        FormKind::Otp => "otp",
        FormKind::Card => "card",
        FormKind::Address => "address",
        FormKind::Identity => "identity",
    }
}

fn is_card(c: FieldClass) -> bool {
    matches!(
        c,
        FieldClass::CcName
            | FieldClass::CcNumber
            | FieldClass::CcExp
            | FieldClass::CcExpMonth
            | FieldClass::CcExpYear
            | FieldClass::CcCsc
    )
}

/// The registrable domain of an origin or URL.
fn site(url: &str) -> Option<String> {
    site_of(url)
}

/// The values an item gives a form's visible fields, by field key.
pub(super) fn values_for(
    item: &Item,
    fields: &[DetectedField],
    now_secs: u64,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for f in fields.iter().filter(|f| f.visible) {
        let v: Option<String> = match (item, f.class) {
            (Item::Login(l), FieldClass::Username | FieldClass::Email) => Some(l.username.clone()),
            (Item::Login(l), FieldClass::Password) => Some(l.password.clone()),
            (Item::Login(l), FieldClass::Otp) => l
                .totp
                .as_deref()
                .and_then(mado_autofill::totp::Totp::parse)
                .map(|t| t.code(now_secs)),
            (Item::Totp(t), FieldClass::Otp) => {
                mado_autofill::totp::Totp::parse(&t.uri).map(|t| t.code(now_secs))
            }
            (Item::Card(c), FieldClass::CcName) => Some(c.name_on_card.clone()),
            (Item::Card(c), FieldClass::CcNumber) => Some(c.number.clone()),
            (Item::Card(c), FieldClass::CcExp) => {
                Some(format!("{:02}/{:02}", c.exp_month, c.exp_year % 100))
            }
            (Item::Card(c), FieldClass::CcExpMonth) => Some(format!("{:02}", c.exp_month)),
            (Item::Card(c), FieldClass::CcExpYear) => Some(c.exp_year.to_string()),
            (Item::Card(c), FieldClass::CcCsc) => c.cvc.clone(),
            (Item::Address(a), class) => {
                let (given, family) = a.name.split_once(' ').unwrap_or((a.name.as_str(), ""));
                match class {
                    FieldClass::Name => Some(a.name.clone()),
                    FieldClass::GivenName => Some(given.to_owned()),
                    FieldClass::FamilyName => Some(family.to_owned()),
                    FieldClass::Organization => a.organization.clone(),
                    FieldClass::AddressLine1 => a.lines.first().cloned(),
                    FieldClass::AddressLine2 => a.lines.get(1).cloned(),
                    FieldClass::AddressLine3 => a.lines.get(2).cloned(),
                    FieldClass::City => Some(a.city.clone()),
                    FieldClass::Region => a.region.clone(),
                    FieldClass::PostalCode => Some(a.postal_code.clone()),
                    FieldClass::Country => Some(a.country.clone()),
                    FieldClass::Tel => a.phone.clone(),
                    FieldClass::Email => a.email.clone(),
                    _ => None,
                }
            }
            (Item::Identity(i), class) => match class {
                FieldClass::GivenName => Some(i.given_name.clone()),
                FieldClass::FamilyName => Some(i.family_name.clone()),
                FieldClass::Name => Some(
                    format!("{} {}", i.given_name, i.family_name)
                        .trim()
                        .to_owned(),
                ),
                FieldClass::Email => i.email.clone(),
                FieldClass::Tel => i.phone.clone(),
                FieldClass::Organization => i.company.clone(),
                FieldClass::Bday => i.birthday.clone(),
                _ => None,
            },
            _ => None,
        };
        if let Some(v) = v.filter(|v| !v.is_empty()) {
            out.insert(f.field_key.clone(), v);
        }
    }
    out
}

/// Which item kinds a form takes.
fn wants(kind: FormKind, item: &Item) -> bool {
    match kind {
        FormKind::Login | FormKind::UsernameOnly | FormKind::ChangePassword | FormKind::Signup => {
            matches!(item, Item::Login(_))
        }
        FormKind::Otp => {
            matches!(item, Item::Login(l) if l.totp.is_some()) || matches!(item, Item::Totp(_))
        }
        FormKind::Card => matches!(item, Item::Card(_)),
        FormKind::Address => matches!(item, Item::Address(_)),
        FormKind::Identity => matches!(item, Item::Identity(_) | Item::Address(_)),
    }
}

/// The fill menu's "Use a strong password" entry (11 §5).
pub(super) const GENERATE: &str = "generate:";

fn generator_item(kind: FormKind) -> Option<FillMenuItem> {
    matches!(kind, FormKind::Signup | FormKind::ChangePassword).then(|| FillMenuItem {
        item_ref: GENERATE.into(),
        title: String::new(),
        subtitle: String::new(),
        kind: "generate".into(),
        source: "mado".into(),
    })
}

fn menu_item(s: &Stored) -> FillMenuItem {
    let subtitle = match &s.item {
        Item::Login(l) => l.username.clone(),
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
            format!("•••• {last}")
        }
        Item::Address(a) => a.lines.first().cloned().unwrap_or_default(),
        Item::Identity(i) => i.email.clone().unwrap_or_default(),
        _ => String::new(),
    };
    FillMenuItem {
        item_ref: format!("mado:{}", s.id),
        title: s.item.title(),
        subtitle,
        kind: serde_json::to_value(s.item.kind())
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        source: "mado".into(),
    }
}

/// The items a form is offered: logins by site, cards, addresses and
/// identities from the whole vault.
pub(super) fn items_for(
    c: &rusqlite::Connection,
    keys: &mado_vault::Keys,
    kind: FormKind,
    site: &str,
) -> Result<Vec<Stored>, VaultError> {
    let found = match kind {
        FormKind::Card | FormKind::Address | FormKind::Identity => Vault::list(c, keys, false)?,
        _ => Vault::find(c, keys, &format!("site:{site}"))?,
    };
    Ok(found.into_iter().filter(|s| wants(kind, &s.item)).collect())
}

impl Core {
    fn gesture_recent(&self) -> bool {
        self.now() - (self.os.last_gesture)() <= GESTURE_MS
    }

    /// The forms a frame reported.
    fn frame_forms(&self, tab: TabId, origin: &str, main: bool) -> Option<&Vec<DetectedForm>> {
        self.page_forms
            .get(&tab)?
            .iter()
            .find(|f| f.origin == origin && f.main == main)
            .map(|f| &f.forms)
    }

    /// 11 §4.1: a focused field opens the fill menu when the frame and the
    /// gesture allow it.
    pub(super) fn field_focused(
        &mut self,
        tab: TabId,
        frame: &FrameInfo,
        f: FieldFocused,
        rec: &mut PatchRecorder,
    ) {
        if self.vault.is_none() || self.state.vault.status == VaultStatus::None {
            return;
        }
        if !self.gesture_recent() {
            tracing::debug!("a field focus without a native gesture");
            return;
        }
        let Some(form) = self
            .frame_forms(tab, &frame.origin, frame.is_main_frame)
            .and_then(|forms| forms.iter().find(|x| x.form_key == f.form_key))
            .cloned()
        else {
            return;
        };
        let Some(field) = form
            .fields
            .iter()
            .find(|x| x.field_key == f.field_key && x.visible)
        else {
            return;
        };
        let Some(tab_url) = self.model.tabs.get(&tab).map(|t| t.url.clone()) else {
            return;
        };
        let top_site = site(&tab_url);
        let frame_site = site(&frame.origin);
        // Subframes: the top frame's own origin, or a payment provider's
        // frame for card fields (11 §4.1 step 2, §12 check 1).
        let top_origin = url::Url::parse(&tab_url).map(|u| u.origin().ascii_serialization());
        let allowed = frame.is_main_frame
            || top_origin.as_deref() == Ok(frame.origin.as_str())
            || (is_card(field.class) && mado_lists::payment_frames().allows(&frame.origin));
        if !allowed {
            tracing::debug!(origin = %frame.origin, "a fill menu refused in a cross-site frame");
            return;
        }
        let Some(site) = (if frame.is_main_frame {
            frame_site
        } else {
            top_site
        }) else {
            return;
        };
        let Some(window_id) = self.model.tabs.get(&tab).map(|t| t.window_id) else {
            return;
        };
        let content = self
            .state
            .windows
            .get(&window_id)
            .and_then(|w| w.content_rect)
            .unwrap_or_default();
        // Subframes report rectangles in their own viewport, whose place in
        // the page Mado does not know: anchor at the page's corner.
        let rect = if frame.is_main_frame {
            Rect {
                x: content.x + f.rect.x,
                y: content.y + f.rect.y,
                width: f.rect.width,
                height: f.rect.height,
            }
        } else {
            Rect {
                x: content.x + 8.0,
                y: content.y + 8.0,
                width: 240.0,
                height: 0.0,
            }
        };
        let target = FillTarget {
            tab,
            frame: frame.handle,
            origin: frame.origin.clone(),
            main: frame.is_main_frame,
            form_key: form.form_key.clone(),
            kind: form.kind,
            site,
            rules: f.password_rules.clone(),
        };
        self.open_fill_menu(window_id, target, rect, None, rec);
    }

    /// Shows the fill menu for `target`, reading its items when unlocked.
    /// `first` is an item to put first (two-step sign-in).
    pub(super) fn open_fill_menu(
        &mut self,
        window_id: mado_types::WindowId,
        target: FillTarget,
        rect: Rect,
        first: Option<String>,
        rec: &mut PatchRecorder,
    ) {
        let mut prompts = self.state.prompts.clone();
        prompts.retain(|_, p| !matches!(p, PromptState::FillMenu(m) if m.window_id == window_id));
        self.fill_targets.retain(|id, _| prompts.contains_key(id));
        let id = PromptId::new();
        let locked = self.vault_keys.is_none();
        prompts.insert(
            id,
            PromptState::FillMenu(FillMenuPrompt {
                id,
                window_id,
                tab_id: target.tab,
                form_key: target.form_key.clone(),
                form_kind: kind_name(target.kind).into(),
                rect,
                locked,
                loading: !locked,
                items: generator_item(target.kind).into_iter().collect(),
            }),
        );
        self.state.tx(rec).set_prompts(prompts);
        if let Some(keys) = self.vault_keys.clone() {
            let (kind, site) = (target.kind, target.site.clone());
            let _ = self.vault_job(None, move |c, _| {
                let mut items: Vec<FillMenuItem> = items_for(c, &keys, kind, &site)?
                    .iter()
                    .map(menu_item)
                    .collect();
                if let Some(first) = first.map(|f| format!("mado:{f}"))
                    && let Some(pos) = items.iter().position(|i| i.item_ref == first)
                {
                    let item = items.remove(pos);
                    items.insert(0, item);
                }
                Ok(VaultOutcome::MenuItems(id, items))
            });
        }
        self.fill_targets.insert(id, target);
    }

    /// The fill menu's items arrived.
    pub(super) fn fill_menu_items(
        &mut self,
        id: PromptId,
        items: Vec<FillMenuItem>,
        rec: &mut PatchRecorder,
    ) {
        let mut prompts = self.state.prompts.clone();
        if let Some(PromptState::FillMenu(m)) = prompts.get_mut(&id) {
            let kind = m.form_kind.clone();
            m.items = items;
            if kind == "signup" || kind == "changePassword" {
                m.items
                    .insert(0, generator_item(FormKind::Signup).expect("an item"));
            }
            m.loading = false;
            self.state.tx(rec).set_prompts(prompts);
        }
    }

    /// After an unlock: menus that said "Unlock to fill" read their items.
    pub(super) fn refresh_fill_menus(&mut self, rec: &mut PatchRecorder) {
        let open: Vec<(PromptId, mado_types::WindowId, Rect)> = self
            .state
            .prompts
            .values()
            .filter_map(|p| match p {
                PromptState::FillMenu(m) if m.locked => Some((m.id, m.window_id, m.rect)),
                _ => None,
            })
            .collect();
        for (id, window, rect) in open {
            if let Some(target) = self.fill_targets.remove(&id) {
                self.open_fill_menu(window, target, rect, None, rec);
            }
        }
    }

    pub(super) fn close_fill_menus(&mut self, tab: TabId, rec: &mut PatchRecorder) {
        if !self
            .state
            .prompts
            .values()
            .any(|p| matches!(p, PromptState::FillMenu(m) if m.tab_id == tab))
        {
            return;
        }
        let mut prompts = self.state.prompts.clone();
        prompts.retain(|_, p| !matches!(p, PromptState::FillMenu(m) if m.tab_id == tab));
        self.fill_targets.retain(|id, _| prompts.contains_key(id));
        self.state.tx(rec).set_prompts(prompts);
    }

    /// `vault.fill` (11 §4.2).
    pub(super) fn vault_fill(
        &mut self,
        origin: &Origin,
        tab: TabId,
        form_key: &str,
        item_ref: &str,
        reply_to: &mut Option<Reply>,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        if !origin.is_user() {
            return Err(MadoError::new(
                ErrorCode::Protected,
                "error.vault.notAllowed",
            ));
        }
        if !self.gesture_recent() {
            return Err(MadoError::new(
                ErrorCode::PermissionDenied,
                "error.vault.noGesture",
            ));
        }
        let target = self
            .fill_targets
            .values()
            .find(|t| t.tab == tab && t.form_key == form_key)
            .cloned()
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.vault.noForm"))?;
        if item_ref == GENERATE {
            return self.fill_generated(target, reply_to, rec);
        }
        let id = item_ref
            .strip_prefix("mado:")
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.vault.notFound"))?
            .to_owned();
        let keys = self.vault_keys()?;
        let approved = self.now() < self.vault_fill_until;
        self.close_fill_menus(tab, rec);
        let reply = reply_to.take();
        let now = self.now();
        self.vault_job(None, move |c, cfg| {
            let stored = Vault::get(c, &keys, &id)?;
            // Cards always ask, and anything once the approval window is
            // over (11 §2.3): the enclave shows Touch ID.
            if matches!(stored.item, Item::Card(_)) || !approved {
                Vault::unlock(c, &*cfg.enclave, &cfg.dir, "fill a password")?;
            }
            Vault::touch(c, &id, now)?;
            Ok(VaultOutcome::Fill {
                target: Box::new(target),
                item: Box::new(stored),
                reply,
            })
        })?;
        Ok(Value::Null)
    }

    /// Sends the values; remembers what follows a login.
    pub(super) fn fill_resolved(
        &mut self,
        target: FillTarget,
        stored: Stored,
        reply: Option<Reply>,
        now: i64,
    ) {
        let Some(form) = self
            .frame_forms(target.tab, &target.origin, target.main)
            .and_then(|f| f.iter().find(|x| x.form_key == target.form_key))
            .cloned()
        else {
            if let Some(r) = reply {
                let _ = r.send(Err(MadoError::new(
                    ErrorCode::NotFound,
                    "error.vault.noForm",
                )));
            }
            return;
        };
        let seconds = u64::try_from(now / 1000).unwrap_or(0);
        let values = values_for(&stored.item, &form.fields, seconds);
        let filled = values.len();
        self.post_fill(&target, values);
        self.vault_fill_until = now
            + self
                .setting("vault.fillApprovalWindow")
                .as_i64()
                .unwrap_or(300_000);
        match target.kind {
            FormKind::UsernameOnly => {
                self.two_step.insert(
                    target.tab,
                    FollowUp {
                        site: target.site.clone(),
                        item: stored.id.clone(),
                        at: now,
                    },
                );
            }
            FormKind::Login => {
                self.otp_after.insert(
                    target.tab,
                    FollowUp {
                        site: target.site.clone(),
                        item: stored.id.clone(),
                        at: now,
                    },
                );
            }
            _ => {}
        }
        if let Some(r) = reply {
            let _ = r.send(Ok(json!({ "filled": filled })));
        }
    }

    /// "Use a strong password": a new password honouring the field's
    /// `passwordrules` goes into the form's new-password fields (and a
    /// sign-up's password field), and is saved when the form succeeds.
    fn fill_generated(
        &mut self,
        target: FillTarget,
        reply_to: &mut Option<Reply>,
        rec: &mut PatchRecorder,
    ) -> Result<Value, MadoError> {
        let form = self
            .frame_forms(target.tab, &target.origin, target.main)
            .and_then(|f| f.iter().find(|x| x.form_key == target.form_key))
            .cloned()
            .ok_or_else(|| MadoError::new(ErrorCode::NotFound, "error.vault.noForm"))?;
        let rules = mado_autofill::generator::parse_rules(target.rules.as_deref().unwrap_or(""));
        let password = mado_autofill::generator::generate(&rules);
        let values: BTreeMap<String, String> = form
            .fields
            .iter()
            .filter(|f| f.visible)
            .filter(|f| {
                f.class == FieldClass::NewPassword
                    || (target.kind == FormKind::Signup && f.class == FieldClass::Password)
            })
            .map(|f| (f.field_key.clone(), password.clone()))
            .collect();
        let filled = values.len();
        self.close_fill_menus(target.tab, rec);
        self.post_fill(&target, values);
        self.generated
            .insert(target.tab, (target.site.clone(), password, self.now()));
        if let Some(r) = reply_to.take() {
            let _ = r.send(Ok(json!({ "filled": filled })));
        }
        Ok(Value::Null)
    }

    fn post_fill(&mut self, target: &FillTarget, values: BTreeMap<String, String>) {
        if values.is_empty() {
            return;
        }
        let message = json!({ "type": "fill", "formKey": target.form_key, "values": values });
        let command = if target.main && target.frame == 0 {
            TabCommand::PostToPage {
                world: World::Mado,
                message,
            }
        } else {
            TabCommand::PostToFrame {
                frame: target.frame,
                world: World::Mado,
                message,
            }
        };
        self.outbox.push(EngineOp::Apply(target.tab, command));
    }

    /// A frame reported new forms: the password step of a two-step sign-in
    /// gets its menu, and a code form after a login gets its code.
    pub(super) fn forms_followed_up(
        &mut self,
        tab: TabId,
        frame: &FrameInfo,
        rec: &mut PatchRecorder,
    ) {
        let now = self.now();
        let Some(forms) = self
            .frame_forms(tab, &frame.origin, frame.is_main_frame)
            .cloned()
        else {
            return;
        };
        let Some(frame_site) = site(&frame.origin) else {
            return;
        };
        if let Some(step) = self.two_step.get(&tab).cloned()
            && now - step.at <= FOLLOW_UP_MS
            && step.site == frame_site
            && let Some(form) = forms.iter().find(|f| f.kind == FormKind::Login)
            && let Some(field) = form
                .fields
                .iter()
                .find(|f| f.class == FieldClass::Password && f.visible)
            && let Some(window_id) = self.model.tabs.get(&tab).map(|t| t.window_id)
        {
            self.two_step.remove(&tab);
            let content = self
                .state
                .windows
                .get(&window_id)
                .and_then(|w| w.content_rect)
                .unwrap_or_default();
            let rect = Rect {
                x: content.x + field.rect.x,
                y: content.y + field.rect.y,
                width: field.rect.width,
                height: field.rect.height,
            };
            let target = FillTarget {
                tab,
                frame: frame.handle,
                origin: frame.origin.clone(),
                main: frame.is_main_frame,
                form_key: form.form_key.clone(),
                kind: form.kind,
                site: frame_site.clone(),
                rules: None,
            };
            self.open_fill_menu(window_id, target, rect, Some(step.item), rec);
        }
        if let Some(login) = self.otp_after.get(&tab).cloned()
            && now - login.at <= FOLLOW_UP_MS
            && login.site == frame_site
            && let Some(form) = forms.iter().find(|f| f.kind == FormKind::Otp)
            && let Some(keys) = self.vault_keys.clone()
        {
            self.otp_after.remove(&tab);
            let target = FillTarget {
                tab,
                frame: frame.handle,
                origin: frame.origin.clone(),
                main: frame.is_main_frame,
                form_key: form.form_key.clone(),
                kind: form.kind,
                site: frame_site,
                rules: None,
            };
            let id = login.item;
            let _ = self.vault_job(None, move |c, _| {
                let stored = Vault::get(c, &keys, &id)?;
                Ok(VaultOutcome::Fill {
                    target: Box::new(target),
                    item: Box::new(stored),
                    reply: None,
                })
            });
        }
    }

    /// The script filled `filled` fields.
    pub(super) fn fill_done(&mut self, tab: TabId, done: &FillDone) {
        tracing::debug!(%tab, filled = done.filled, form = %done.form_key, "a form was filled");
    }
}
