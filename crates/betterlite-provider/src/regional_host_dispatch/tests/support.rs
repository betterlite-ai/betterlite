use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use secrecy::{ExposeSecret as _, SecretString};

use crate::regional_host_dispatch::chat::dispatch;
use crate::regional_host_dispatch::context::CallLogging;
use crate::regional_host_dispatch::error::DispatchError;

mod chat;
mod chat_context;
mod chat_recorder;
mod embedding;
mod embedding_context;
mod speech;
mod speech_context;
mod speech_recorder;
mod tokens;
mod transport;

pub(super) use chat::ChatFixture;
pub(super) use chat_recorder::ChatRecorder;
pub(super) use embedding::EmbeddingFixture;
pub(super) use speech::SpeechFixture;
pub(super) use speech_recorder::{SpeechRecorder, SpeechSnapshot};
pub(super) use tokens::{FailingTokens, FakeTokens};
pub(super) use transport::FakeTransport;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).expect("the fixture init parses"),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .expect("the fixture logging state builds");
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags {
            log_raw_request_response: true,
            turn_off_message_logging: false,
        },
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .expect("the fixture time parses"),
    };
    (state, env)
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the fixture redactor builds")
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

pub(super) fn secret_text(value: Option<&SecretString>) -> Option<String> {
    value.map(|key| key.expose_secret().to_owned())
}

pub(super) fn params(source: &str) -> Object {
    serde_json::from_str(source).expect("the fixture params parse")
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn user_message() -> Message {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("the fixture message parses")
}

// run one chat arm with fresh logging state, and hand the state back so a test can read what it logged
pub(super) fn chat_dispatch(
    tag: ChatConfigTag,
    fixture: &ChatFixture,
    handler: &ChatRecorder,
) -> (Result<String, DispatchError>, LoggingState) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let outcome = {
        let mut log = CallLogging {
            state: &mut state,
            env: &env,
        };
        dispatch(tag, &fixture.context(), handler, &mut log)
            .expect("the regional table has this arm")
    };
    (outcome, state)
}

pub(super) fn proxy_policy() -> ProxyPolicy {
    ProxyPolicy {
        no_proxy: String::new(),
        primary: BTreeMap::new(),
        fallback: BTreeMap::new(),
        warnings: Vec::new(),
    }
}
