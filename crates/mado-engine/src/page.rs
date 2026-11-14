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
    UnsavedInput,
    FillDone,
    WebauthnCreate,
    WebauthnGet,
    NotificationRequest,
    NotificationShow,
    NotificationClose,
    GeolocationGet,
    GeolocationWatch,
    GeolocationClear,
    ProtocolHandlerRegister,
    MediaState,
    DrmFailed,
    ReaderProbe,
    ReaderContent,
    CosmeticClasses,
    PickerResult,
    Favicons,
    PageText,
    ContextTarget,
    StoreInstall,
    AiSnapshot,
    AiText,
    AiActionResult,
    Invoke,
);

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
    decode(&envelope.kind, &envelope.script, envelope.payload)
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
}
