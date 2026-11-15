//! Page scripts and their messages (spec 02 §5): the embedded bundles, the
//! per-tab nonce, and validation of every message a page script sends.
//! Anything that fails validation is dropped and counted; the sender frame's
//! origin always comes from the engine, never from the message.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// The `bridge` bundle, with `__MADO_NONCE__` where the nonce goes.
const BRIDGE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/bridge.js"
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

empty_payloads!(
    Forms,
    FieldFocused,
    CredentialsSubmitted,
    FillDone,
    WebauthnCreate,
    WebauthnGet,
    NotificationRequest,
    ReaderProbe,
    ReaderContent,
    CosmeticClasses,
    PickerResult,
    StoreInstall,
    AiSnapshot,
    AiText,
    AiActionResult,
    Invoke,
);

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
    ReaderProbe(ReaderProbe) = "readerProbe" from ["reader"],
    ReaderContent(ReaderContent) = "readerContent" from ["reader"],
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
        _ => true,
    }
}

const STATUS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/status.js"
));
const CONTEXT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/context.js"
));
const FIND: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/find.js"
));
const MEDIA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/media.js"
));
const FORMS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/forms.js"
));
const RELAY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/relay.js"
));
const NOTIFICATIONS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/notifications.js"
));
const GEOLOCATION: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/geolocation.js"
));
const APPLE_PAY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/apple-pay.js"
));
const PROTOCOL_HANDLERS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/protocol-handlers.js"
));

const READER_PROBE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/reader-probe.js"
));
const READER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/reader.js"
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

const GPC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../app/resources/inject/gpc.js"
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
        s("notifications", Page, DocumentStart, true, NOTIFICATIONS),
        s(
            "protocol-handlers",
            Page,
            DocumentStart,
            true,
            PROTOCOL_HANDLERS,
        ),
        s("apple-pay", Page, DocumentStart, true, APPLE_PAY),
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
                "notifications",
                "protocol-handlers",
                "apple-pay"
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
