use std::fmt::Debug;
use std::mem;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::responses_id_security::Settings;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher,
    StoredValue,
};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

pub(super) const SALT: &str = "test-salt";
pub(super) const FUZZ_SALT: &str = "fuzz-salt";

pub(super) type TestCipher = StoredCipher<CountingNonces, TracingDiagnostics>;

pub(super) struct CountingNonces(AtomicU64);

// a deterministic nonce stream, so two fixtures differ only when their counters start apart
impl NonceSource for CountingNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        let count = self.0.fetch_add(1, Ordering::SeqCst);
        let Some((head, tail)) = nonce.split_first_chunk_mut::<8>() else {
            return Err(SecretError::Truncated);
        };
        head.copy_from_slice(&count.to_be_bytes());
        for byte in tail.iter_mut() {
            *byte = 0;
        }
        Ok(())
    }
}

pub(super) fn fixture(
    disable: bool,
    allow: bool,
    key: Option<&str>,
    nonce_start: u64,
) -> (Settings, TestCipher) {
    let signing_key = key.map(SecretString::from);
    let settings = Settings {
        disable_responses_id_security: disable,
        allow_unmanaged_response_ids: allow,
        signing_key: signing_key.clone(),
    };
    let cipher = StoredCipher {
        settings: CipherSettings {
            signing_key,
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: CountingNonces(AtomicU64::new(nonce_start)),
        diagnostics: TracingDiagnostics,
    };
    (settings, cipher)
}

pub(super) fn principal(user: Option<&str>, team: Option<&str>, roles: &[&str]) -> Principal {
    let user = user.map_or_else(|| "null".to_owned(), |id| format!(r#"{{"id":"{id}"}}"#));
    let teams = team.map_or_else(|| "[]".to_owned(), |id| format!(r#"[{{"id":"{id}"}}]"#));
    let roles = roles
        .iter()
        .map(|role| format!(r#""{role}""#))
        .collect::<Vec<String>>()
        .join(",");
    let source = format!(
        r#"{{"principal_type":"human","subject":"subject","auth_method":"api_key","user":{user},"teams":{teams},"roles":[{roles}]}}"#
    );
    serde_json::from_str(&source).unwrap()
}

pub(super) fn data(entries: Vec<(&str, &str)>) -> IndexMap<String, Json> {
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), Json::Text(value.to_owned())))
        .collect()
}

// the wire format LiteLLM writes, built here from literals so a renamed constant shows up as a failure
pub(super) fn managed_id(cipher: &TestCipher, original: &str, user: &str, team: &str) -> String {
    let payload =
        format!("litellm_proxy:responses_api:response_id:{original};user_id:{user};team_id:{team}");
    advertised(cipher, &payload)
}

pub(super) fn advertised(cipher: &TestCipher, payload: &str) -> String {
    let stored = StoredValue::<()>::Text(SecretString::from(payload.to_owned()));
    let sealed = cipher.encrypt_value(stored, None).unwrap();
    let StoredValue::Text(text) = sealed else {
        panic!("the cipher returned a non-text value");
    };
    format!("resp_{}", text.expose_secret())
}

// decrypts the same segment LiteLLM feeds the cipher: the text between the first and second resp_
pub(super) fn unsealed(cipher: &TestCipher, advertised_id: &str) -> String {
    let Some(segment) = advertised_id.split("resp_").nth(1) else {
        panic!("no resp_ in {advertised_id}");
    };
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Original,
    };
    let stored = StoredValue::<()>::Text(SecretString::from(segment.to_owned()));
    let opened = cipher.decrypt_value(stored, "response_id", policy).unwrap();
    let StoredValue::Text(text) = opened else {
        panic!("the cipher returned a non-text value");
    };
    text.expose_secret().to_owned()
}

pub(super) fn response(source: &str) -> ResponsesApiResponse {
    serde_json::from_str(source).unwrap()
}

pub(super) fn stream_event(source: &str) -> ResponsesAPIStreamingResponse {
    serde_json::from_str(source).unwrap()
}

pub(super) fn lifecycle_event(response_id: &str) -> ResponsesAPIStreamingResponse {
    let source = format!(
        r#"{{"type":"response.created","sequence_number":0,"response":{{"id":"{response_id}","created_at":0,"output":[]}}}}"#
    );
    serde_json::from_str(&source).unwrap()
}

pub(super) fn json_of<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
}

pub(super) fn at<'json>(value: &'json Json, path: &[&str]) -> &'json Json {
    let mut current = value;
    for name in path {
        let Json::Object(fields) = current else {
            panic!("{name}: not inside an object");
        };
        current = fields
            .get(*name)
            .unwrap_or_else(|| panic!("missing field {name}"));
    }
    current
}

pub(super) fn text_of(value: &Json) -> &str {
    let Json::Text(text) = value else {
        panic!("expected text, found {value:?}");
    };
    text
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<String>>>);

impl CapturedLogs {
    pub(super) fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => mem::take(&mut messages),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedLogs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture<T, F>(call: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, call);
    (value, logs.take())
}
