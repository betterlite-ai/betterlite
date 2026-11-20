use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::hosted_platform_keys::Settings as SectionSettings;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use secrecy::SecretString;

use crate::hosted_platform_dispatch::call::CompletionCall;
use crate::hosted_platform_dispatch::context::DispatchSettings;
use crate::hosted_platform_dispatch::handler::{ChatCompletionHandler, HandlerError};

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

fn user_message() -> Message {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("the fixture message parses")
}

pub(super) struct Fixture {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    pub(super) optional_params: Object,
    pub(super) litellm_params: Object,
    pub(super) groq_config: Object,
    pub(super) provider: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) stream: Option<bool>,
    pub(super) acompletion: bool,
    pub(super) timeout: Option<Duration>,
    pub(super) provider_config: Option<ChatConfigTag>,
    pub(super) settings: SectionSettings,
    pub(super) input: EmbeddingInput,
    pub(super) aembedding: bool,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            model: String::from("model"),
            messages: vec![user_message()],
            optional_params: Object::new(),
            litellm_params: Object::new(),
            groq_config: Object::new(),
            provider: String::from("fireworks_ai"),
            api_base: None,
            api_key: None,
            headers: None,
            stream: None,
            acompletion: false,
            timeout: None,
            provider_config: None,
            settings: SectionSettings::default(),
            input: EmbeddingInput::Text(String::from("hello")),
            aembedding: false,
        }
    }

    pub(super) fn settings_view(&self) -> DispatchSettings<'_> {
        DispatchSettings::from(&self.settings)
    }
}

pub(super) struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    failure: Option<u16>,
}

impl Recorder {
    pub(super) const fn succeeding() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: None,
        }
    }

    pub(super) const fn failing(status_code: u16) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Some(status_code),
        }
    }

    pub(super) fn first(&self) -> CompletionCall {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .expect("the arm called the handler")
    }
}

impl ChatCompletionHandler for Recorder {
    type Reply = String;

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        self.failure.map_or_else(
            || Ok(String::from("reply")),
            |status_code| Err(HandlerError::Upstream { status_code }),
        )
    }
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

pub(super) fn secret_text(value: Option<&SecretString>) -> Option<String> {
    use secrecy::ExposeSecret as _;
    value.map(|key| key.expose_secret().to_owned())
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}
