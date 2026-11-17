use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::compatible_embedding_settings;
use betterlite_config::sections::openai_like_settings;
use betterlite_config::sections::self_hosted_dispatch::Settings as SectionSettings;
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

use crate::self_hosted_dispatch::call::{CompletionCall, EmbeddingCall};
use crate::self_hosted_dispatch::context::{DispatchContext, DispatchSettings, EmbeddingContext};
use crate::self_hosted_dispatch::handler::{ChatCompletionHandler, HandlerError};

use super::section::SectionFixture;

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
        || "trace".to_owned(),
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

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn user_message() -> Message {
    serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("the fixture message parses")
}

pub(super) struct Fixture {
    pub(super) model: String,
    pub(super) messages: Vec<Message>,
    pub(super) optional_params: Object,
    pub(super) litellm_params: Object,
    pub(super) provider: String,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) headers: Option<BTreeMap<String, String>>,
    pub(super) stream: Option<bool>,
    pub(super) acompletion: bool,
    pub(super) timeout: Option<Duration>,
    pub(super) settings: SectionSettings,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) globals: compatible_embedding_settings::Globals,
    pub(super) compatible: compatible_embedding_settings::Settings,
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
            provider: String::from("hosted_vllm"),
            api_base: None,
            api_key: None,
            headers: None,
            stream: None,
            acompletion: false,
            timeout: None,
            settings: SectionFixture::default().into_settings(),
            environment: BTreeMap::new(),
            globals: compatible_embedding_settings::Globals::default(),
            compatible: compatible_embedding_settings::Settings::default(),
            input: EmbeddingInput::Text(String::from("hello")),
            aembedding: false,
        }
    }

    pub(super) fn configure(&mut self, section: SectionFixture) {
        self.settings = section.into_settings();
    }

    pub(super) fn openai_like(
        &mut self,
        api_base: Option<&str>,
        api_key: Option<&str>,
    ) -> &mut Self {
        self.compatible.openai_like = openai_like_settings::Settings {
            api_base: api_base.map(str::to_owned),
            api_key: api_key.map(SecretString::from),
        };
        self
    }

    pub(super) fn chat_context(&self) -> DispatchContext<'_> {
        DispatchContext {
            acompletion: self.acompletion,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            custom_llm_provider: &self.provider,
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

    pub(super) fn embedding_context(&self) -> EmbeddingContext<'_> {
        EmbeddingContext {
            settings: DispatchSettings::from(&self.settings),
            environment: &self.environment,
            compatible: &self.compatible,
            globals: &self.globals,
        }
    }

    pub(super) fn embedding_call(&self) -> EmbeddingCall {
        EmbeddingCall {
            model: self.model.clone(),
            input: self.input.clone(),
            optional_params: self.optional_params.clone(),
            litellm_params: self.litellm_params.clone(),
            api_key: self.api_key.clone(),
            api_base: self.api_base.clone(),
            headers: self.headers.clone(),
            timeout: self.timeout.unwrap_or(Duration::from_secs(600)),
            aembedding: self.aembedding,
        }
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

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

pub(super) fn secret_text(value: Option<&SecretString>) -> Option<String> {
    use secrecy::ExposeSecret as _;
    value.map(|key| key.expose_secret().to_owned())
}
