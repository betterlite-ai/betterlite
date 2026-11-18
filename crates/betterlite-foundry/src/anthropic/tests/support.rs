use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_anthropic::anthropic_capability_catalog::{CatalogSources, DropParams};
use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
use betterlite_anthropic::anthropic_http_stream::error::{PostFailure, StatusFailure};
use betterlite_anthropic::anthropic_http_stream::transport::{
    LineChunks, StreamPost, StreamReply, StreamTransport,
};
use betterlite_anthropic::capabilities::speed::{SpeedParam, SpeedRequest, maybe_drop_speed_param};
use betterlite_anthropic::chat_call::config::{
    ChatConfig, TransformOutcome, TransformRequest, TransformResponse,
};
use betterlite_anthropic::chat_call::error::{ClassifiedConfigError, ConfigError};
use betterlite_anthropic::chat_call::transport::{BufferedReply, ChatTransport};
use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::request::BillingHeader;
use betterlite_anthropic::request::beta_headers::add_context_management_beta;
use betterlite_anthropic::request::system::translate_system_message;
use betterlite_anthropic::response::error::ResponseError;
use betterlite_anthropic::response::{Logging as ResponseLogging, transform_response};
use betterlite_anthropic::stream_decode::TransportChunk;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;
use uuid::Uuid;

pub(super) const MODEL: &str = "claude-sonnet-4-5";
pub(super) const API_BASE: &str = "https://resource.services.ai.azure.com/anthropic/v1/messages";
pub(super) const CALLER_KEY: &str = "sk-ant-caller-0123456789";
pub(super) const CONFIGURED_KEY: &str = "sk-ant-configured-9876543210";
pub(super) const TOKEN: &str = "fixture-entra-token-0123456789";
pub(super) const NOW: i64 = 1_767_225_600;
pub(super) const TIMEOUT: Duration = Duration::from_secs(1);

const INIT: &str = r#"{"model":"claude-sonnet-4-5","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1","messages":[],"litellm_params":{},"request_logging":{}}"#;
const ENV_NOW: &str = "2026-01-01T00:00:05+00:00";

#[derive(Debug, Error)]
#[error("{message}")]
pub(super) struct FixtureFailure {
    pub(super) message: String,
}

#[derive(Clone)]
pub(super) enum Trip {
    Reply,
    Unknown(&'static str),
    Status {
        status_code: u16,
        headers: HeaderMap,
        body: Vec<u8>,
        // the handler prefers exception.headers over response.headers, and a masked httpx error has only the latter
        exception_headers: bool,
    },
}

#[derive(Clone)]
pub(super) struct FixtureReply {
    pub(super) status_code: u16,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
    pub(super) lines: Vec<String>,
}

impl FixtureReply {
    pub(super) fn text(content: &str) -> Self {
        Self {
            status_code: 200,
            headers: HeaderMap::new(),
            body: text_body(content).into_bytes(),
            lines: vec![
                "event: message_start".to_owned(),
                "data: {\"type\":\"message_start\"}".to_owned(),
            ],
        }
    }
}

pub(super) fn text_body(content: &str) -> String {
    format!(
        r#"{{"id":"msg_1","type":"message","role":"assistant","model":"{MODEL}","content":[{{"type":"text","text":"{content}"}}],"stop_reason":"end_turn","stop_sequence":null,"usage":{{"input_tokens":3,"output_tokens":2}}}}"#
    )
}

pub(super) struct FixtureTransport {
    pub(super) reply: FixtureReply,
    pub(super) trip: Trip,
    posts: Mutex<Vec<StreamPost>>,
}

impl FixtureTransport {
    pub(super) fn new(reply: FixtureReply, trip: Trip) -> Self {
        Self {
            reply,
            trip,
            posts: Mutex::new(Vec::new()),
        }
    }

    fn all(&self) -> MutexGuard<'_, Vec<StreamPost>> {
        self.posts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, post: StreamPost) {
        self.all().push(post);
    }

    pub(super) fn posted_bodies(&self) -> Vec<IndexMap<String, Json>> {
        self.all()
            .iter()
            .map(|post| serde_json::from_slice(&post.body).unwrap())
            .collect()
    }

    pub(super) fn posted_headers(&self) -> Vec<HeaderMap> {
        self.all().iter().map(|post| post.headers.clone()).collect()
    }

    pub(super) fn posted_urls(&self) -> Vec<String> {
        self.all().iter().map(|post| post.url.to_string()).collect()
    }

    pub(super) fn posted_deadlines(&self) -> Vec<Duration> {
        self.all().iter().map(|post| post.deadline).collect()
    }

    pub(super) fn posts(&self) -> usize {
        self.all().len()
    }
}

fn trip_failure(trip: &Trip) -> Option<PostFailure<FixtureFailure>> {
    match trip {
        Trip::Reply => None,
        Trip::Unknown(message) => Some(PostFailure::Unknown(FixtureFailure {
            message: (*message).to_owned(),
        })),
        Trip::Status {
            status_code,
            headers,
            body,
            exception_headers,
        } => Some(PostFailure::Status(Box::new(StatusFailure {
            status_code: *status_code,
            body: body.clone(),
            exception_headers: exception_headers.then(|| headers.clone()),
            response_headers: (!exception_headers).then(|| headers.clone()),
        }))),
    }
}

impl StreamTransport for FixtureTransport {
    type Failure = FixtureFailure;

    fn post(
        &self,
        post: StreamPost,
    ) -> impl Future<Output = Result<StreamReply<FixtureFailure>, PostFailure<FixtureFailure>>> + Send
    {
        self.record(post);
        if let Some(failure) = trip_failure(&self.trip) {
            return ready(Err(failure));
        }
        let lines: Vec<String> = self.reply.lines.clone();
        let decoded: LineChunks<FixtureFailure> =
            stream::iter(lines.into_iter().map(|line| Ok(TransportChunk::Text(line)))).boxed();
        ready(Ok(StreamReply {
            status_code: self.reply.status_code,
            headers: self.reply.headers.clone(),
            lines: decoded,
        }))
    }
}

impl ChatTransport for FixtureTransport {
    fn post_buffered(
        &self,
        post: StreamPost,
    ) -> impl Future<Output = Result<BufferedReply, PostFailure<FixtureFailure>>> + Send {
        self.record(post);
        ready(trip_failure(&self.trip).map_or_else(
            || {
                Ok(BufferedReply {
                    status_code: self.reply.status_code,
                    headers: self.reply.headers.clone(),
                    body: self.reply.body.clone(),
                })
            },
            Err,
        ))
    }
}

pub(super) struct Recorded {
    pub(super) optional_params: IndexMap<String, Json>,
    pub(super) headers: IndexMap<String, String>,
    pub(super) custom_llm_provider: Option<String>,
    pub(super) billing_header: BillingHeader,
}

pub(super) struct ResponseRecord {
    pub(super) json_mode: Option<bool>,
    pub(super) optional_params: IndexMap<String, Json>,
    pub(super) has_vertex_request: bool,
}

pub(super) struct FixtureConfig {
    pub(super) body: Option<IndexMap<String, Json>>,
    pub(super) transform_failure: Option<String>,
    pub(super) transforms: Mutex<Vec<Recorded>>,
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) responses: Mutex<Vec<ResponseRecord>>,
}

impl FixtureConfig {
    // The inherited mapper reads the provider identity and billing rule off its request, then maps the prompt
    fn map(
        &self,
        request: &TransformRequest<'_>,
        mode: &str,
    ) -> Result<TransformOutcome, ConfigError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(mode.to_owned());
        self.transforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                optional_params: request.optional_params.clone(),
                headers: request.headers.values.clone(),
                custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
                billing_header: request.billing_header,
            });
        if let Some(message) = &self.transform_failure {
            let error: ConfigError = Box::new(FixtureFailure {
                message: message.clone(),
            });
            return Err(error);
        }
        reject_unsupported_speed(request)?;
        let headers = mapped_headers(request)?;
        let mut body = self
            .body
            .clone()
            .unwrap_or_else(|| request.optional_params.clone());
        if let Some(system) = mapped_system(request)? {
            body.insert("system".to_owned(), system);
        }
        Ok(TransformOutcome { body, headers })
    }
}

// The parent transform rejects speed for any provider but anthropic unless drop_params is on
fn reject_unsupported_speed(request: &TransformRequest<'_>) -> Result<(), ConfigError> {
    let Some(speed) = request.optional_params.get("speed") else {
        return Ok(());
    };
    let patterns = CandidatePatterns::new().map_err(ConfigError::from)?;
    let runtime = RawCatalog::new();
    let packaged = RawCatalog::new();
    let catalogs = CatalogSources {
        patterns: &patterns,
        runtime: &runtime,
        packaged: &packaged,
    };
    let call = SpeedRequest {
        model: request.model,
        speed: SpeedParam::Present(speed),
        drop: DropParams::default(),
        custom_llm_provider: request.custom_llm_provider,
    };
    match maybe_drop_speed_param(&call, &catalogs) {
        Ok(_) => Ok(()),
        Err(error) => {
            let failure: ConfigError = match error.classify() {
                Some(class) => Box::new(ClassifiedConfigError {
                    source: Box::new(error),
                    class,
                }),
                None => Box::new(error),
            };
            Err(failure)
        }
    }
}

// The parent transform adds the optional betas to the headers it was handed, and this override sends them on
fn mapped_headers(request: &TransformRequest<'_>) -> Result<AnthropicHeaders, ConfigError> {
    let mut headers = request.headers.clone();
    if let Some(context_management) = request.optional_params.get("context_management") {
        add_context_management_beta(&mut headers, context_management).map_err(ConfigError::from)?;
    }
    Ok(headers)
}

// The parent turns the system run into the top-level prompt, dropping billing blocks when it was told to
fn mapped_system(request: &TransformRequest<'_>) -> Result<Option<Json>, ConfigError> {
    let mut messages: Vec<Message> = request
        .messages
        .iter()
        .map(decode::<Message>)
        .collect::<Result<_, _>>()?;
    let blocks = translate_system_message(
        &mut messages,
        request.billing_header == BillingHeader::Strip,
    );
    if blocks.is_empty() {
        return Ok(None);
    }
    encode(&blocks).map(Some)
}

// The message records and this JSON tree differ only in their field shapes, so one serde pass converts either way
fn decode<Target>(value: &Json) -> Result<Target, ConfigError>
where
    Target: DeserializeOwned,
{
    let plain = serde_json::to_value(value).map_err(ConfigError::from)?;
    serde_json::from_value(plain).map_err(ConfigError::from)
}

fn encode<Source>(value: &Source) -> Result<Json, ConfigError>
where
    Source: Serialize,
{
    let plain = serde_json::to_value(value).map_err(ConfigError::from)?;
    serde_json::from_value(plain).map_err(ConfigError::from)
}

impl ChatConfig for FixtureConfig {
    fn transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> Result<IndexMap<String, Json>, ConfigError> {
        Ok(self.map(request, "sync")?.body)
    }

    fn async_transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> impl Future<Output = Result<IndexMap<String, Json>, ConfigError>> + Send {
        let mapped = self.map(request, "async").map(|outcome| outcome.body);
        ready(mapped)
    }

    fn transform_outcome(
        &self,
        request: &TransformRequest<'_>,
    ) -> Result<TransformOutcome, ConfigError> {
        self.map(request, "sync")
    }

    fn filter_request_betas(
        &self,
        provider: &str,
        _headers: &mut IndexMap<String, String>,
        _body: &mut IndexMap<String, Json>,
    ) -> Result<(), ConfigError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("filter:{provider}"));
        Ok(())
    }

    fn transform_response<Counter>(
        &self,
        response: TransformResponse<'_, '_, '_>,
        logging: ResponseLogging<'_>,
        counter: &Counter,
    ) -> Result<(), ResponseError<Counter::Failure>>
    where
        Counter: ReasoningTokenCounter,
    {
        let TransformResponse {
            envelope,
            raw,
            request,
        } = response;
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ResponseRecord {
                json_mode: request.json_mode,
                optional_params: request.optional_params.clone(),
                has_vertex_request: request.optional_params.contains_key("is_vertex_request"),
            });
        transform_response(envelope, raw, request, logging, counter)
    }
}

pub(super) struct LoggerCall {
    pub(super) input: Json,
    pub(super) original_response: String,
    pub(super) complete_input_dict: String,
}

#[derive(Default)]
pub(super) struct RecordingLogger {
    pub(super) calls: Mutex<Vec<LoggerCall>>,
}

impl StreamLogger for RecordingLogger {
    fn post_call(&self, call: StreamPostCall<'_>) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LoggerCall {
                input: call.input.clone(),
                original_response: call.original_response.to_owned(),
                complete_input_dict: call.complete_input_dict.to_owned(),
            });
    }
}

#[derive(Clone, Copy)]
pub(super) struct FixtureCounter;

impl ReasoningTokenCounter for FixtureCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

pub(super) struct FixtureIds;

impl IdSource for FixtureIds {
    fn next_id(&self) -> Uuid {
        Uuid::nil()
    }
}

pub(super) struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

// Every token source refuses, so a case that reaches one fails the way an offline host does
pub(super) struct Offline;

fn refuse<T>(source: &str) -> Result<T, SourceFailure> {
    Err(SourceFailure {
        class: "ValueError".to_owned(),
        status: None,
        message: format!("the {source} source is offline"),
    })
}

impl CredentialSources for Offline {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("client secret")
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("username and password")
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        refuse("SDK refresh")
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        refuse("OIDC")
    }
}

pub(super) fn logging_state(redactor: &Redactor) -> LoggingState {
    let init = serde_json::from_str::<LoggingInit>(INIT).unwrap();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .unwrap()
}

pub(super) fn logging_env<'env>(redactor: &'env Redactor, sink: &'env Silent) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::<FixedOffset>::parse_from_rfc3339(ENV_NOW).unwrap(),
    }
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

pub(super) fn messages() -> Vec<Json> {
    vec![Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text("user".to_owned())),
        ("content".to_owned(), Json::Text("hi".to_owned())),
    ]))]
}

pub(super) fn body() -> IndexMap<String, Json> {
    let mut body = IndexMap::new();
    body.insert("model".to_owned(), Json::Text(MODEL.to_owned()));
    body.insert(
        "max_tokens".to_owned(),
        Json::Number(serde_json::Number::from(64_i64)),
    );
    body.insert("messages".to_owned(), Json::List(messages()));
    body
}

pub(super) fn headers_of(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    headers
}
