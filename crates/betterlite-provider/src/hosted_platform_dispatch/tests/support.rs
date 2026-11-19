use std::collections::BTreeMap;
use std::future::{Future, ready};
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
use betterlite_openai::fireworks_ai_chat_params::FireworksCapabilities;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use secrecy::SecretString;

use crate::hosted_platform_dispatch::call::{CompletionCall, EmbeddingCall};
use crate::hosted_platform_dispatch::chat::dispatch;
use crate::hosted_platform_dispatch::context::{CallLogging, DispatchContext, DispatchSettings};
use crate::hosted_platform_dispatch::error::DispatchError;
use crate::hosted_platform_dispatch::handler::{ChatCompletionHandler, HandlerError};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

pub(super) struct Capabilities {
    pub(super) reasoning: bool,
}

impl FireworksCapabilities for Capabilities {
    fn supports_function_calling(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_tool_choice(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_reasoning(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(self.reasoning)
    }
}

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

    pub(super) fn context(&self) -> DispatchContext<'_> {
        DispatchContext {
            acompletion: self.acompletion,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            custom_llm_provider: &self.provider,
            groq_config: &self.groq_config,
            headers: self.headers.as_ref(),
            litellm_params: &self.litellm_params,
            messages: &self.messages,
            model: &self.model,
            optional_params: &self.optional_params,
            provider_config: self.provider_config,
            stream: self.stream,
            timeout: self.timeout,
            settings: DispatchSettings::from(&self.settings),
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
            timeout: self.timeout.unwrap_or_default(),
            aembedding: self.aembedding,
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

pub(super) async fn dispatch_chat(
    tag: ChatConfigTag,
    fixture: &Fixture,
    handler: &Recorder,
    log: &mut CallLogging<'_, '_>,
) -> Result<String, DispatchError> {
    let context = fixture.context();
    let mut capabilities = Capabilities { reasoning: true };
    dispatch(tag, &context, &mut capabilities, handler, log)
        .await
        .expect("the hosted table has this arm")
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
