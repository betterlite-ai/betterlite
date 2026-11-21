use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::gateway_provider_keys::Settings as SectionSettings;
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
use secrecy::SecretString;

use crate::gateway_provider_dispatch::call::CompletionCall;
use crate::gateway_provider_dispatch::context::{DispatchContext, DispatchSettings};
use crate::gateway_provider_dispatch::handler::{ChatCompletionHandler, HandlerError};

const INIT: &str = r#"{"model":"oracle","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

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
        || "trace".to_owned(),
        Duration::default(),
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
    SecretString::from(String::from(value))
}

pub(super) fn secret_text(value: Option<&SecretString>) -> Option<String> {
    use secrecy::ExposeSecret as _;
    value.map(|key| key.expose_secret().to_owned())
}

pub(super) fn object(value: impl serde::Serialize) -> Object {
    serde_json::from_value(serde_json::to_value(value).expect("the fixture value serializes"))
        .expect("the fixture object parses")
}

fn user_message() -> Message {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("the fixture message parses")
}

pub(super) struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    failure: Option<u16>,
}

impl Recorder {
    pub(super) fn succeeding() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: None,
        }
    }

    pub(super) fn failing(status: u16) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Some(status),
        }
    }

    pub(super) fn calls(&self) -> Vec<CompletionCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

// the request and section values one arm reads
pub(super) struct Fixture {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    pub(super) provider: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) stream: Option<bool>,
    pub(super) timeout: Option<Duration>,
    pub(super) optional_params: Object,
    pub(super) litellm_params: Object,
    pub(super) class_config: Object,
    pub(super) settings: SectionSettings,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self {
            model: String::from("model"),
            messages: vec![user_message()],
            provider: String::from("openrouter"),
            api_base: None,
            api_key: None,
            headers: None,
            stream: None,
            timeout: None,
            optional_params: Object::new(),
            litellm_params: Object::new(),
            class_config: Object::new(),
            settings: SectionSettings::default(),
        }
    }

    pub(super) fn context(&self) -> DispatchContext<'_> {
        DispatchContext {
            acompletion: false,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            custom_llm_provider: &self.provider,
            class_config: &self.class_config,
            headers: self.headers.as_ref(),
            litellm_params: &self.litellm_params,
            messages: &self.messages,
            model: &self.model,
            optional_params: &self.optional_params,
            stream: self.stream,
            timeout: self.timeout,
            settings: DispatchSettings::from(&self.settings),
        }
    }
}
