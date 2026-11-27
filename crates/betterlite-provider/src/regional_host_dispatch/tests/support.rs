use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_http::gigachat_tokens::error::GigachatAuthError;
use betterlite_http::gigachat_tokens::request::GigachatTokenRequest;
use betterlite_http::gigachat_tokens::tokens::GigachatTokens;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::gigachat_files::GigachatFileTransport;
use betterlite_openai::openai_base_config::Object as SpeechObject;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::speech_http::SpeechCall;
use crate::api::speech_provider_contract::{SpeechHeaders, TextToSpeechConfig};
use crate::hosted_platform_dispatch::call::CompletionCall;
use crate::hosted_platform_dispatch::handler::{ChatCompletionHandler, HandlerError};
use crate::regional_host_dispatch::context::{DispatchContext, DispatchSettings, EmbeddingContext};
use crate::regional_host_dispatch::error::DispatchError;
use crate::regional_host_dispatch::speech::{SpeechContext, SpeechHandler};

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

pub(super) struct ChatFixture {
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
    pub(super) provider_config: Option<ChatConfigTag>,
    pub(super) settings: SectionSettings,
}

impl ChatFixture {
    pub(super) fn new() -> Self {
        Self {
            model: String::from("model"),
            messages: vec![user_message()],
            optional_params: Object::new(),
            litellm_params: Object::new(),
            provider: String::from("nlp_cloud"),
            api_base: None,
            api_key: None,
            headers: None,
            stream: None,
            acompletion: false,
            timeout: None,
            provider_config: None,
            settings: SectionSettings::default(),
        }
    }

    pub(super) fn context(&self) -> DispatchContext<'_> {
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
            provider_config: self.provider_config,
            stream: self.stream,
            timeout: self.timeout,
            settings: DispatchSettings::from(&self.settings),
        }
    }
}

pub(super) struct EmbeddingFixture {
    pub(super) input: EmbeddingInput,
    pub(super) optional_params: Object,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) extra_headers: Option<BTreeMap<String, String>>,
    pub(super) ssl_verify: Option<bool>,
    pub(super) settings: SectionSettings,
}

impl EmbeddingFixture {
    pub(super) fn new() -> Self {
        Self {
            input: EmbeddingInput::Text(String::from("hello")),
            optional_params: Object::new(),
            api_base: None,
            api_key: None,
            extra_headers: None,
            ssl_verify: None,
            settings: SectionSettings::default(),
        }
    }

    pub(super) fn context(&self) -> EmbeddingContext<'_> {
        EmbeddingContext {
            aembedding: false,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            extra_headers: self.extra_headers.as_ref(),
            input: &self.input,
            model: "embed-model",
            optional_params: &self.optional_params,
            ssl_verify: self.ssl_verify,
            timeout: Duration::from_secs(30),
            settings: DispatchSettings::from(&self.settings),
        }
    }
}

pub(super) struct SpeechFixture {
    pub(super) input: String,
    pub(super) model: String,
    pub(super) voice: Option<SpeechVoice>,
    pub(super) optional_params: Object,
    pub(super) litellm_params: Object,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<SecretString>,
    pub(super) extra_headers: Option<SpeechHeaders>,
    pub(super) ssl_verify: Option<bool>,
    pub(super) timeout: Option<Duration>,
    pub(super) settings: SectionSettings,
}

impl SpeechFixture {
    pub(super) fn new() -> Self {
        Self {
            input: String::from("hello"),
            model: String::from("speech-02-hd"),
            voice: None,
            optional_params: Object::new(),
            litellm_params: Object::new(),
            api_base: None,
            api_key: None,
            extra_headers: None,
            ssl_verify: None,
            timeout: None,
            settings: SectionSettings::default(),
        }
    }

    pub(super) fn context(&self) -> SpeechContext<'_> {
        SpeechContext {
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            extra_headers: self.extra_headers.as_ref(),
            input: &self.input,
            litellm_params: &self.litellm_params,
            model: &self.model,
            optional_params: &self.optional_params,
            ssl_verify: self.ssl_verify,
            timeout: self.timeout,
            voice: self.voice.clone(),
            settings: DispatchSettings::from(&self.settings),
        }
    }
}

pub(super) struct ChatRecorder {
    calls: Mutex<Vec<CompletionCall>>,
    failure: Option<u16>,
}

impl ChatRecorder {
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

impl ChatCompletionHandler for ChatRecorder {
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

#[derive(Debug, Clone)]
pub(super) struct SpeechSnapshot {
    pub(super) model: String,
    pub(super) input: String,
    pub(super) voice: Option<String>,
    pub(super) optional_params: SpeechObject,
    pub(super) litellm_params: SpeechObject,
    pub(super) extra_headers: Option<SpeechHeaders>,
    pub(super) api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) ssl_verify: Option<bool>,
    pub(super) timeout: Option<Duration>,
}

pub(super) struct SpeechRecorder {
    calls: Mutex<Vec<SpeechSnapshot>>,
}

impl SpeechRecorder {
    pub(super) const fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn first(&self) -> SpeechSnapshot {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .expect("the arm called the handler")
    }

    pub(super) fn calls(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl SpeechHandler for SpeechRecorder {
    type Reply = String;

    fn synthesize(
        &self,
        call: &SpeechCall<'_>,
        _config: &dyn TextToSpeechConfig,
    ) -> impl Future<Output = Result<Self::Reply, DispatchError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(snapshot(call));
        ready(Ok(String::from("spoken")))
    }
}

fn snapshot(call: &SpeechCall<'_>) -> SpeechSnapshot {
    SpeechSnapshot {
        model: call.model.to_owned(),
        input: call.input.to_owned(),
        voice: call.voice.map(str::to_owned),
        optional_params: call.optional_params.clone(),
        litellm_params: call.litellm_params.clone(),
        extra_headers: call.extra_headers.cloned(),
        api_key: call
            .params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        api_base: call.params.api_base.clone(),
        ssl_verify: call.params.ssl_verify,
        timeout: call.timeout,
    }
}

pub(super) struct FakeTokens {
    requests: Mutex<Vec<GigachatTokenRequest>>,
}

impl FakeTokens {
    pub(super) const fn new() -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn request(&self) -> GigachatTokenRequest {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .expect("the bearer asked the service")
    }
}

#[async_trait]
impl GigachatTokens for FakeTokens {
    async fn access_token(
        &self,
        request: &GigachatTokenRequest,
    ) -> Result<SecretString, GigachatAuthError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(secret("token-1"))
    }
}

pub(super) struct FailingTokens;

#[async_trait]
impl GigachatTokens for FailingTokens {
    async fn access_token(
        &self,
        _request: &GigachatTokenRequest,
    ) -> Result<SecretString, GigachatAuthError> {
        Err(GigachatAuthError::MissingCredentials)
    }
}

pub(super) struct FakeTransport {
    uploads: Arc<AtomicUsize>,
}

impl FakeTransport {
    pub(super) fn new() -> Self {
        Self {
            uploads: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn counter(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.uploads)
    }
}

impl GigachatFileTransport for FakeTransport {
    fn upload(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.uploads.fetch_add(1, Ordering::SeqCst);
        ready(Ok(UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: br#"{"id":"file-1"}"#.to_vec(),
        }))
    }
}

pub(super) fn proxy_policy() -> ProxyPolicy {
    ProxyPolicy {
        no_proxy: String::new(),
        primary: BTreeMap::new(),
        fallback: BTreeMap::new(),
        warnings: Vec::new(),
    }
}
