//! Page scripts and their messages (spec 02 §5): the embedded bundles, the
//! per-tab nonce, and validation of every message a page script sends.
//! Anything that fails validation is dropped and counted; the sender frame's
//! origin always comes from the engine, never from the message.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `bridge` bundle, with `__MADO_NONCE__` where the nonce goes.
const BRIDGE: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/bridge.js"
));
const NONCE_PLACEHOLDER: &str = "__MADO_NONCE__";

/// A random value generated per tab per navigation (spec 02 §5.2).
#[derive(Clone, PartialEq, Eq)]
pub struct Nonce(String);

impl std::fmt::Debug for Nonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Nonce(..)")
    }
}

impl Nonce {
    /// 128 random bits as hex.
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)?;
        Ok(Self(bytes.iter().map(|b| format!("{b:02x}")).collect()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn matches(&self, other: &str) -> bool {
        self.0.len() == other.len()
            && self
                .0
                .bytes()
                .zip(other.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }
}

/// The `bridge` script for one tab and navigation, nonce baked in.
pub fn bridge_source(nonce: &Nonce) -> String {
    BRIDGE.replace(NONCE_PLACEHOLDER, nonce.as_str())
}

/// Why a message was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Dropped {
    /// Not an object with `v`, `nonce`, `script`, `type` and `payload`.
    Malformed,
    UnknownVersion,
    WrongNonce,
    UnknownType,
    /// A known type sent by a script that may not send it.
    WrongScript,
    /// The payload does not match its type.
    InvalidPayload,
}

/// Counts of dropped messages, per reason.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DropCounts(pub BTreeMap<Dropped, u64>);

impl DropCounts {
    pub fn add(&mut self, reason: Dropped) {
        *self.0.entry(reason).or_default() += 1;
    }

    pub fn total(&self) -> u64 {
        self.0.values().sum()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    v: u32,
    nonce: String,
    script: String,
    #[serde(rename = "type")]
    kind: String,
    payload: Value,
}

macro_rules! messages {
    ($( $variant:ident ( $payload:ident ) = $name:literal from [$($script:literal),+], )*) => {
        /// Every message type of spec 02 §5.0, with its validated payload.
        /// A payload type with no fields yet accepts only `{}`: the task that
        /// implements the handler defines its fields.
        #[derive(Debug, Clone, PartialEq)]
        pub enum PageMessage {
            $($variant($payload),)*
        }

        impl PageMessage {
            /// The wire name, as in spec 02 §5.0.
            pub fn name(&self) -> &'static str {
                match self { $(Self::$variant(_) => $name,)* }
            }
        }

        fn decode(kind: &str, script: &str, payload: Value) -> Result<PageMessage, Dropped> {
            match kind {
                $($name => {
                    if ![$($script),+].contains(&script) {
                        return Err(Dropped::WrongScript);
                    }
                    parse::<$payload>(payload).map(PageMessage::$variant)
                })*
                _ => Err(Dropped::UnknownType),
            }
        }
    };
}

fn parse<T: DeserializeOwned>(payload: Value) -> Result<T, Dropped> {
    serde_json::from_value(payload).map_err(|_| Dropped::InvalidPayload)
}

macro_rules! empty_payloads {
    ($($name:ident),* $(,)?) => {$(
        #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {}
    )*};
}

/// `webauthn`: `navigator.credentials.create` (spec 11 §8.2). Byte
/// strings are base64url.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WebauthnCreate {
    pub id: u32,
    pub rp_id: Option<String>,
    pub rp_name: String,
    pub user_id: String,
    pub user_name: String,
    pub user_display_name: String,
    pub challenge: String,
    pub algs: Vec<i64>,
    pub exclude: Vec<String>,
}

/// `webauthn`: `navigator.credentials.get` (spec 11 §8.3).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WebauthnGet {
    pub id: u32,
    pub rp_id: Option<String>,
    pub challenge: String,
    pub allow: Vec<String>,
    pub conditional: bool,
}

/// `webauthn`: the page aborted request `id`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebauthnCancel {
    pub id: u32,
}

empty_payloads!(NotificationRequest, AiSnapshot, AiText, AiActionResult,);

/// `internal`: a `mado://` page sends a command (spec 01 §4.4). The core
/// checks the sending frame and the page's allow-list before parsing it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invoke {
    pub id: u64,
    pub command: Value,
}

/// `forms`: the frame's forms Mado could fill (spec 11 §3). Keys are the
/// script's own random IDs; rectangles are in the frame's viewport.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Forms {
    pub forms: Vec<DetectedForm>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DetectedForm {
    pub form_key: String,
    pub kind: FormKind,
    pub action: Option<String>,
    pub fields: Vec<DetectedField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FormKind {
    Login,
    UsernameOnly,
    Signup,
    ChangePassword,
    Otp,
    Card,
    Address,
    Identity,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DetectedField {
    pub field_key: String,
    pub class: FieldClass,
    pub rect: FieldRect,
    pub visible: bool,
    pub in_viewport: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A field's class (spec 11 §3), as `inject/src/lib/formdetect.ts` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldClass {
    Username,
    Email,
    Password,
    NewPassword,
    Otp,
    CcName,
    CcNumber,
    CcExp,
    CcExpMonth,
    CcExpYear,
    CcCsc,
    GivenName,
    FamilyName,
    Name,
    Organization,
    AddressLine1,
    AddressLine2,
    AddressLine3,
    City,
    Region,
    PostalCode,
    Country,
    Tel,
    Bday,
}

/// `forms`: a trusted focus or mouse-down on a detected field (spec 11
/// §4.1); `rect` in the frame's viewport.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FieldFocused {
    pub form_key: String,
    pub field_key: String,
    pub rect: FieldRect,
    /// The field's `passwordrules` attribute (spec 11 §5), for the
    /// generator.
    #[serde(default)]
    pub password_rules: Option<String>,
}

/// `forms`: a form was submitted with these values (spec 11 §5 step 1).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CredentialsSubmitted {
    pub form_key: String,
    pub kind: FormKind,
    pub username: String,
    pub password: String,
}

/// `forms`: how many fields a fill set (spec 11 §4.2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FillDone {
    pub form_key: String,
    pub filled: u32,
}

/// `forms`: whether editable fields hold typed text (spec 04 §7, §9).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsavedInput {
    pub dirty: bool,
}

/// `notifications`: `new Notification(title, options)` (spec 07 §1.1).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationShow {
    pub id: u32,
    pub title: String,
    pub body: String,
    pub tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationClose {
    pub id: u32,
}

/// `geolocation`: one position, or a watch (spec 07 §1.1).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GeolocationGet {
    pub id: u32,
    pub high_accuracy: bool,
}

pub type GeolocationWatch = GeolocationGet;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeolocationClear {
    pub id: u32,
}

/// `protocol-handlers`: `navigator.registerProtocolHandler` (spec 07 §6).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolHandlerRegister {
    pub scheme: String,
    pub url: String,
}

/// `forms`: a search form was submitted on a page that links an OpenSearch
/// description (spec 05 §3, discovery).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenSearch {
    pub href: String,
}

/// `store`: "Add to Mado" or "Remove from Mado" on a Chrome Web Store
/// detail page (spec 09 §2.1). Only the ID is trusted, and only after the
/// core checks it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreInstall {
    pub id: String,
    #[serde(default)]
    pub remove: bool,
}

/// `cosmetic`: class and id names seen on the page since the last batch
/// (spec 10 §6 step 3), at most 1,000 each.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CosmeticClasses {
    pub classes: Vec<String>,
    pub ids: Vec<String>,
}

/// `cosmetic`: the element picker's choice (spec 10 §8): a selector to
/// hide on the page's host, or nothing when cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickerResult {
    pub selector: Option<String>,
}

/// `apple-pay`: a checkout asked whether Apple Pay works (spec 07 §9).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplePayProbe {}

/// `media`: for the media button (spec 05 §11).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaState {
    pub playing: bool,
    pub muted: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
}

/// `media`: an encrypted stream got no key (spec 05 §11).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DrmFailed {
    pub key_system: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Icon {
    pub url: String,
    pub sizes: Option<String>,
    pub rel: String,
}

/// `status`: the page's icons.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Favicons {
    pub icons: Vec<Icon>,
}

/// `status`: the page's text for history search (spec 12 §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageText {
    pub text: String,
}

/// `context`: the element under the pointer (spec 07 §8).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ContextTarget {
    pub link: Option<String>,
    pub link_text: Option<String>,
    pub image: Option<String>,
    pub media: Option<String>,
    pub media_kind: Option<String>,
    pub selection: Option<String>,
    pub editable: bool,
}

/// `status`: the page is alive (spec 02 §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Heartbeat {}

/// `status`: the link under the pointer, for the status bubble.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoverLink {
    pub url: Option<String>,
}

/// `find`: match count and active match (spec 02, `FindResult`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindResult {
    pub matches: Option<u32>,
    pub active: Option<u32>,
}

messages! {
    Forms(Forms) = "forms" from ["forms"],
    FieldFocused(FieldFocused) = "fieldFocused" from ["forms"],
    CredentialsSubmitted(CredentialsSubmitted) = "credentialsSubmitted" from ["forms"],
    UnsavedInput(UnsavedInput) = "unsavedInput" from ["forms"],
    OpenSearch(OpenSearch) = "openSearch" from ["forms"],
    FillDone(FillDone) = "fillDone" from ["forms"],
    WebauthnCreate(WebauthnCreate) = "webauthnCreate" from ["webauthn"],
    WebauthnGet(WebauthnGet) = "webauthnGet" from ["webauthn"],
    WebauthnCancel(WebauthnCancel) = "webauthnCancel" from ["webauthn"],
    NotificationRequest(NotificationRequest) = "notificationRequest" from ["notifications"],
    NotificationShow(NotificationShow) = "notificationShow" from ["notifications"],
    NotificationClose(NotificationClose) = "notificationClose" from ["notifications"],
    GeolocationGet(GeolocationGet) = "geolocationGet" from ["geolocation"],
    GeolocationWatch(GeolocationWatch) = "geolocationWatch" from ["geolocation"],
    GeolocationClear(GeolocationClear) = "geolocationClear" from ["geolocation"],
    ProtocolHandlerRegister(ProtocolHandlerRegister) = "protocolHandlerRegister" from ["protocol-handlers"],
    ApplePayProbe(ApplePayProbe) = "applePayProbe" from ["apple-pay"],
    MediaState(MediaState) = "mediaState" from ["media"],
    DrmFailed(DrmFailed) = "drmFailed" from ["media"],
    FindResult(FindResult) = "findResult" from ["find"],
    CosmeticClasses(CosmeticClasses) = "cosmeticClasses" from ["cosmetic"],
    PickerResult(PickerResult) = "pickerResult" from ["cosmetic"],
    HoverLink(HoverLink) = "hoverLink" from ["status"],
    Favicons(Favicons) = "favicons" from ["status"],
    Heartbeat(Heartbeat) = "heartbeat" from ["status"],
    PageText(PageText) = "pageText" from ["status"],
    ContextTarget(ContextTarget) = "contextTarget" from ["context"],
    StoreInstall(StoreInstall) = "storeInstall" from ["store"],
    AiSnapshot(AiSnapshot) = "aiSnapshot" from ["ai-reader"],
    AiText(AiText) = "aiText" from ["ai-reader"],
    AiActionResult(AiActionResult) = "aiActionResult" from ["ai-reader"],
    Invoke(Invoke) = "invoke" from ["internal"],
}

/// Checks one message from a tab whose current nonce is `nonce`.
pub fn validate(raw: &Value, nonce: &Nonce) -> Result<PageMessage, Dropped> {
    let envelope: Envelope = serde_json::from_value(raw.clone()).map_err(|_| Dropped::Malformed)?;
    if envelope.v != 1 {
        return Err(Dropped::UnknownVersion);
    }
    if !nonce.matches(&envelope.nonce) {
        return Err(Dropped::WrongNonce);
    }
    let message = decode(&envelope.kind, &envelope.script, envelope.payload)?;
    within_limits(&message)
        .then_some(message)
        .ok_or(Dropped::InvalidPayload)
}

/// Size limits the scripts keep; anything bigger came from elsewhere.
fn within_limits(m: &PageMessage) -> bool {
    let short = |s: &str| s.len() <= 4096;
    let opt = |s: &Option<String>| s.as_deref().is_none_or(short);
    match m {
        PageMessage::HoverLink(h) => opt(&h.url),
        PageMessage::Favicons(f) => {
            f.icons.len() <= 16
                && f.icons
                    .iter()
                    .all(|i| short(&i.url) && opt(&i.sizes) && i.rel.len() <= 64)
        }
        PageMessage::PageText(p) => p.text.len() <= 200 * 1024 * 4,
        PageMessage::ContextTarget(c) => {
            opt(&c.link)
                && opt(&c.link_text)
                && opt(&c.image)
                && opt(&c.media)
                && opt(&c.selection)
                && c.media_kind
                    .as_deref()
                    .is_none_or(|k| k == "video" || k == "audio")
        }
        PageMessage::MediaState(s) => opt(&s.title) && opt(&s.artist),
        PageMessage::NotificationShow(n) => {
            n.title.len() <= 1024 && n.body.len() <= 4096 && n.tag.len() <= 1024
        }
        PageMessage::ProtocolHandlerRegister(r) => r.scheme.len() <= 64 && r.url.len() <= 2048,
        PageMessage::DrmFailed(d) => d.key_system.as_deref().is_none_or(|k| k.len() <= 128),
        PageMessage::OpenSearch(o) => o.href.len() <= 2048 && o.href.starts_with("https://"),
        PageMessage::Invoke(i) => i.command.to_string().len() <= 64 * 1024,
        PageMessage::FillDone(d) => d.form_key.len() <= 64,
        PageMessage::WebauthnCreate(w) => {
            let b64 = |s: &str| {
                s.len() <= 1024
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            };
            opt(&w.rp_id)
                && w.rp_name.len() <= 1024
                && w.user_name.len() <= 1024
                && w.user_display_name.len() <= 1024
                && b64(&w.user_id)
                && b64(&w.challenge)
                && w.algs.len() <= 16
                && w.exclude.len() <= 64
                && w.exclude.iter().all(|c| b64(c))
        }
        PageMessage::WebauthnGet(w) => {
            let b64 = |s: &str| {
                s.len() <= 1024
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            };
            opt(&w.rp_id)
                && b64(&w.challenge)
                && w.allow.len() <= 64
                && w.allow.iter().all(|c| b64(c))
        }
        PageMessage::CredentialsSubmitted(c) => {
            c.form_key.len() <= 64
                && (c.kind == FormKind::UsernameOnly || !c.password.is_empty())
                && c.password.len() <= 1024
                && c.username.len() <= 1024
        }
        PageMessage::FieldFocused(f) => {
            let key =
                |k: &str| (1..=64).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_hexdigit());
            key(&f.form_key)
                && key(&f.field_key)
                && opt(&f.password_rules)
                && [f.rect.x, f.rect.y, f.rect.width, f.rect.height]
                    .iter()
                    .all(|v| v.is_finite())
        }
        PageMessage::Forms(f) => {
            let key =
                |k: &str| (1..=64).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_hexdigit());
            let finite =
                |r: &FieldRect| [r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite());
            f.forms.len() <= 20
                && f.forms.iter().all(|form| {
                    key(&form.form_key)
                        && opt(&form.action)
                        && form.fields.len() <= 50
                        && form
                            .fields
                            .iter()
                            .all(|x| key(&x.field_key) && finite(&x.rect))
                })
        }
        _ => true,
    }
}

const STATUS: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/status.js"
));
const CONTEXT: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/context.js"
));
const FIND: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/find.js"
));
const MEDIA: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/media.js"
));
const FORMS: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/forms.js"
));
const RELAY: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/relay.js"
));
const WEBAUTHN: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/webauthn.js"
));

/// `webauthn` (spec 11 §8.1), added while Mado Passwords handles passkeys.
pub fn webauthn() -> crate::PageScript {
    crate::PageScript {
        name: "webauthn",
        world: crate::World::Page,
        at: crate::InjectAt::DocumentStart,
        main_frame_only: true,
        source: WEBAUTHN.to_owned(),
    }
}

const NOTIFICATIONS: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/notifications.js"
));
const GEOLOCATION: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/geolocation.js"
));
const COSMETIC: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/cosmetic.js"
));
const STORE: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/store.js"
));
const APPLE_PAY: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/apple-pay.js"
));
const PROTOCOL_HANDLERS: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/protocol-handlers.js"
));

const AI_READER: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/ai-reader.js"
));

/// The body for `EvaluateInWorld` that answers the page's readable text
/// for AI (spec 13 §5): `{ title, url, text, readable, sensitiveField }`.
/// Starts with `// ai-text`, which tests match.
pub fn ai_text() -> String {
    format!("// ai-text\n{AI_READER}\nreturn globalThis[Symbol.for(\"mado.ai\")].text();")
}

/// The body for `EvaluateInWorld` that calls `ai-reader`'s `call` with
/// JSON `args` (spec 13 §8): `snapshot`, `locate`, `focus`, `choose`,
/// `scroll` or `quiet`. Starts with `// ai-<call>`, which tests match.
pub fn ai_call(call: &str, args: &serde_json::Value) -> String {
    let args: Vec<String> = match args {
        serde_json::Value::Array(a) => a.iter().map(serde_json::Value::to_string).collect(),
        other => vec![other.to_string()],
    };
    format!(
        "// ai-{call}\n{AI_READER}\nreturn globalThis[Symbol.for(\"mado.ai\")][{}]({});",
        serde_json::Value::String(call.to_owned()),
        args.join(", ")
    )
}

const READER_PROBE: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/reader-probe.js"
));
const READER: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/reader.js"
));

/// Where the `reader` scripts leave their answer in the Mado world.
const READER_RESULT: &str = r#"return globalThis[Symbol.for("mado.readerResult")];"#;

/// The body for `EvaluateInWorld` that answers whether the page looks like
/// an article (spec 08 §10).
pub fn reader_probe() -> String {
    format!("// reader-probe\n{READER_PROBE}\n{READER_RESULT}")
}

/// The body for `EvaluateInWorld` that answers the article, or null.
pub fn reader_extract() -> String {
    format!("// reader-extract\n{READER}\n{READER_RESULT}")
}

const INTERNAL: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/internal.js"
));
const GPC: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/inject/gpc.js"
));

/// `gpc`, added to the bundled set while `privacy.gpc` is on (spec 08 §8).
pub fn gpc() -> crate::PageScript {
    crate::PageScript {
        name: "gpc",
        world: crate::World::Page,
        at: crate::InjectAt::DocumentStart,
        main_frame_only: false,
        source: GPC.to_owned(),
    }
}

/// The page scripts M2 injects in every tab after its `bridge` (spec 02 §5
/// table). `geolocation_shim`: before macOS 27, where WebKit has no
/// geolocation permission delegate (spec 07 §1.1).
pub fn bundled(geolocation_shim: bool) -> crate::PageScriptSet {
    use crate::{InjectAt::*, PageScript, World::*};
    let s = |name: &'static str, world, at, main_frame_only, source: &str| PageScript {
        name,
        world,
        at,
        main_frame_only,
        source: source.to_owned(),
    };
    let mut scripts = vec![
        s("status", Mado, DocumentStart, true, STATUS),
        s("context", Mado, DocumentStart, false, CONTEXT),
        s("find", Mado, DocumentEnd, true, FIND),
        s("media", Mado, DocumentStart, false, MEDIA),
        s("forms", Mado, DocumentEnd, false, FORMS),
        s("relay", Mado, DocumentStart, true, RELAY),
        s("internal", Mado, DocumentStart, true, INTERNAL),
        s("notifications", Page, DocumentStart, true, NOTIFICATIONS),
        s(
            "protocol-handlers",
            Page,
            DocumentStart,
            true,
            PROTOCOL_HANDLERS,
        ),
        s("apple-pay", Page, DocumentStart, true, APPLE_PAY),
        s("store", Mado, DocumentEnd, true, STORE),
        s("cosmetic", Mado, DocumentStart, false, COSMETIC),
    ];
    if geolocation_shim {
        scripts.push(s("geolocation", Page, DocumentStart, true, GEOLOCATION));
    }
    crate::PageScriptSet { scripts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(nonce: &str, script: &str, kind: &str, payload: Value) -> Value {
        json!({ "v": 1, "nonce": nonce, "script": script, "type": kind, "payload": payload })
    }

    #[test]
    fn valid_messages_pass() {
        let n = Nonce::generate().expect("random");
        assert_eq!(
            validate(&message(n.as_str(), "status", "heartbeat", json!({})), &n),
            Ok(PageMessage::Heartbeat(Heartbeat {}))
        );
        let found = validate(
            &message(
                n.as_str(),
                "find",
                "findResult",
                json!({ "matches": 3, "active": 1 }),
            ),
            &n,
        )
        .expect("valid");
        assert_eq!(found.name(), "findResult");
    }

    #[test]
    fn forged_wrong_nonce_and_malformed_messages_are_dropped() {
        let n = Nonce::generate().expect("random");
        let other = Nonce::generate().expect("random");
        let mut counts = DropCounts::default();
        let cases = [
            (
                message(other.as_str(), "status", "heartbeat", json!({})),
                Dropped::WrongNonce,
            ),
            (
                message("", "status", "heartbeat", json!({})),
                Dropped::WrongNonce,
            ),
            // Forged: a type only the forms script may send, claimed by status.
            (
                message(n.as_str(), "status", "credentialsSubmitted", json!({})),
                Dropped::WrongScript,
            ),
            (
                message(n.as_str(), "page", "invoke", json!({})),
                Dropped::WrongScript,
            ),
            (
                message(n.as_str(), "status", "nukeEverything", json!({})),
                Dropped::UnknownType,
            ),
            (
                message(
                    n.as_str(),
                    "find",
                    "findResult",
                    json!({ "matches": "lots" }),
                ),
                Dropped::InvalidPayload,
            ),
            (
                message(n.as_str(), "status", "heartbeat", json!({ "extra": 1 })),
                Dropped::InvalidPayload,
            ),
            (
                json!({ "v": 2, "nonce": n.as_str(), "script": "status", "type": "heartbeat", "payload": {} }),
                Dropped::UnknownVersion,
            ),
            (json!("heartbeat"), Dropped::Malformed),
            (
                json!({ "v": 1, "nonce": n.as_str(), "type": "heartbeat", "payload": {} }),
                Dropped::Malformed,
            ),
            (
                json!({ "v": 1, "nonce": n.as_str(), "script": "status", "type": "heartbeat", "payload": {}, "origin": "https://bank.test" }),
                Dropped::Malformed,
            ),
        ];
        for (raw, reason) in cases {
            let result = validate(&raw, &n);
            assert_eq!(result, Err(reason), "{raw}");
            counts.add(reason);
        }
        assert_eq!(counts.total(), 11);
    }

    #[test]
    fn the_bridge_carries_this_tab_s_nonce_and_no_placeholder() {
        let n = Nonce::generate().expect("random");
        let source = bridge_source(&n);
        assert!(source.contains(n.as_str()));
        assert!(!source.contains(NONCE_PLACEHOLDER));
        assert!(
            BRIDGE.matches(NONCE_PLACEHOLDER).count() == 1,
            "one place for the nonce"
        );
        assert!(source.contains("mado.bridge"));
    }

    #[test]
    fn nonces_are_random_and_long() {
        let a = Nonce::generate().expect("random");
        let b = Nonce::generate().expect("random");
        assert_ne!(a, b);
        assert_eq!(a.as_str().len(), 32);
    }

    /// `forms` (spec 11 §3): known classes and kinds only, hex keys, and
    /// bounded counts.
    #[test]
    fn detected_forms_validate() {
        let nonce = Nonce::generate().expect("nonce");
        let msg = |payload: serde_json::Value| json!({ "v": 1, "nonce": nonce.as_str(), "script": "forms", "type": "forms", "payload": payload });
        let field = |key: &str, class: &str| json!({ "fieldKey": key, "class": class, "rect": { "x": 1.0, "y": 2.0, "width": 200.0, "height": 30.0 }, "visible": true, "inViewport": true });
        let form = |fields: Vec<serde_json::Value>| json!({ "forms": [{ "formKey": "a1b2", "kind": "login", "action": "https://a.example/session", "fields": fields }] });
        let Ok(PageMessage::Forms(f)) = validate(
            &msg(form(vec![field("c3", "username"), field("d4", "password")])),
            &nonce,
        ) else {
            panic!("valid forms");
        };
        assert_eq!(f.forms[0].kind, FormKind::Login);
        assert_eq!(f.forms[0].fields[1].class, FieldClass::Password);
        for bad in [
            form(vec![field("c3", "secretSauce")]),
            form(vec![field("not hex", "username")]),
            json!({ "forms": [{ "formKey": "a1", "kind": "bank", "action": null, "fields": [] }] }),
            form(
                (0..51)
                    .map(|i| field(&format!("{i:x}"), "unknownClass"))
                    .collect(),
            ),
            form((0..51).map(|i| field(&format!("{i:x}"), "email")).collect()),
        ] {
            assert!(validate(&msg(bad), &nonce).is_err());
        }
    }

    #[test]
    fn m2_payloads_validate_and_oversize_ones_are_dropped() {
        let nonce = Nonce::generate().expect("nonce");
        let msg = |script: &str, kind: &str, payload: serde_json::Value| json!({ "v": 1, "nonce": nonce.as_str(), "script": script, "type": kind, "payload": payload });
        assert!(matches!(
            validate(
                &msg("forms", "unsavedInput", json!({ "dirty": true })),
                &nonce
            ),
            Ok(PageMessage::UnsavedInput(UnsavedInput { dirty: true }))
        ));
        assert!(validate(&msg("status", "favicons", json!({ "icons": [{ "url": "https://a/x.png", "sizes": "32x32", "rel": "icon" }] })), &nonce).is_ok());
        assert!(
            validate(
                &msg(
                    "notifications",
                    "notificationShow",
                    json!({ "id": 1, "title": "t", "body": "", "tag": "" })
                ),
                &nonce
            )
            .is_ok()
        );
        assert!(
            validate(
                &msg(
                    "protocol-handlers",
                    "protocolHandlerRegister",
                    json!({ "scheme": "web+x", "url": "https://a/?u=%s" })
                ),
                &nonce
            )
            .is_ok()
        );
        let long = "x".repeat(5000);
        assert_eq!(
            validate(&msg("status", "hoverLink", json!({ "url": long })), &nonce),
            Err(Dropped::InvalidPayload)
        );
        assert_eq!(
            validate(
                &msg("forms", "unsavedInput", json!({ "dirty": "yes" })),
                &nonce
            ),
            Err(Dropped::InvalidPayload)
        );
        let many: Vec<_> = (0..17)
            .map(|i| json!({ "url": format!("https://a/{i}"), "sizes": null, "rel": "icon" }))
            .collect();
        assert_eq!(
            validate(&msg("status", "favicons", json!({ "icons": many })), &nonce),
            Err(Dropped::InvalidPayload)
        );
    }

    #[test]
    fn the_bundled_scripts_follow_the_table() {
        let set = bundled(false);
        let names: Vec<&str> = set.scripts.iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "status",
                "context",
                "find",
                "media",
                "forms",
                "relay",
                "internal",
                "notifications",
                "protocol-handlers",
                "apple-pay",
                "store",
                "cosmetic"
            ]
        );
        assert!(set.scripts.iter().all(|s| !s.source.is_empty()));
        assert!(gpc().source.contains("globalPrivacyControl"));
        assert!(reader_probe().ends_with("readerResult\")];"));
        assert!(reader_extract().len() > 10_000, "Readability is bundled");
        assert_eq!(
            bundled(true).scripts.last().map(|s| s.name),
            Some("geolocation")
        );
    }
}
