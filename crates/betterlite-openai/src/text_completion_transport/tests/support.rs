use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, StreamOptions};
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::text_completion_policy::transform_completion_request;

use super::super::logging::{
    CompletionLogger, PostCallRecord, PreCallRecord, SecretHeaders, StreamDefaults,
};
use super::super::sdk::{
    ByteStream, ClientConstructor, ClientKind, RawCompletion, SdkArguments, SdkError, StatusError,
    TextCompletionSdk,
};

pub(super) type Log = Arc<Mutex<Vec<Value>>>;

pub(super) fn log() -> Log {
    Arc::new(Mutex::new(Vec::new()))
}

pub(super) fn entries(log: &Log) -> Vec<Value> {
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

pub(super) fn push(log: &Log, value: Value) {
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(value);
}

pub(super) fn object(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )
}

pub(super) fn text(value: &str) -> Value {
    Value::String(value.to_owned())
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::try_from(*name).expect("header name");
        let header_value = HeaderValue::try_from(*value).expect("header value");
        map.insert(header_name, header_value);
    }
    map
}

#[derive(Debug, Clone)]
pub(super) struct Failure {
    pub status: Option<u16>,
    pub message: String,
    pub headers: Vec<(String, String)>,
    pub error_type: Option<String>,
    pub param: Option<String>,
    pub code: Option<String>,
}

impl Failure {
    pub(super) fn status(status: u16) -> Self {
        Self {
            status: Some(status),
            message: "Error code: provider failed".to_owned(),
            headers: Vec::new(),
            error_type: None,
            param: None,
            code: None,
        }
    }

    pub(super) fn other(message: &str) -> Self {
        Self {
            status: None,
            message: message.to_owned(),
            headers: Vec::new(),
            error_type: None,
            param: None,
            code: None,
        }
    }

    pub(super) fn error(&self) -> SdkError {
        self.status.map_or_else(
            || SdkError::Other {
                message: self.message.clone(),
            },
            |status| {
                SdkError::Status(Box::new(StatusError {
                    status: Some(status),
                    message: self.message.clone(),
                    headers: headers(
                        &self
                            .headers
                            .iter()
                            .map(|(name, value)| (name.as_str(), value.as_str()))
                            .collect::<Vec<_>>(),
                    ),
                    error_type: self.error_type.clone(),
                    param: self.param.clone(),
                    code: self.code.clone(),
                }))
            },
        )
    }
}

#[derive(Clone)]
pub(super) enum Answer {
    Buffered {
        status: u16,
        body: String,
    },
    Stream {
        events: Vec<String>,
        failure: Option<Failure>,
    },
    CreateFailure(Failure),
    NullStatus {
        message: String,
    },
}

impl Answer {
    pub(super) fn buffered(body: &Value) -> Self {
        Self::Buffered {
            status: 200,
            body: body.to_string(),
        }
    }

    pub(super) fn status(status: u16, body: &Value) -> Self {
        Self::Buffered {
            status,
            body: body.to_string(),
        }
    }

    pub(super) fn stream(events: Vec<Value>) -> Self {
        Self::Stream {
            events: events.into_iter().map(|event| event.to_string()).collect(),
            failure: None,
        }
    }

    pub(super) fn stream_failure(events: Vec<Value>, failure: Failure) -> Self {
        Self::Stream {
            events: events.into_iter().map(|event| event.to_string()).collect(),
            failure: Some(failure),
        }
    }
}

fn sse(events: &[String], failure: Option<&Failure>) -> ByteStream {
    let mut items: Vec<Result<Vec<u8>, SdkError>> = events
        .iter()
        .map(|event| Ok(format!("data: {event}\n\n").into_bytes()))
        .collect();
    match failure {
        Some(error) => items.push(Err(error.error())),
        None => items.push(Ok(b"data: [DONE]\n\n".to_vec())),
    }
    Box::pin(stream::iter(items))
}

#[derive(Clone)]
pub(super) struct FakeSdk {
    pub kind: ClientKind,
    pub calls: Log,
    pub answer: Answer,
}

impl TextCompletionSdk for FakeSdk {
    fn kind(&self) -> ClientKind {
        self.kind
    }

    fn create(
        &self,
        data: &Map<String, Value>,
    ) -> impl Future<Output = Result<RawCompletion, SdkError>> + Send {
        push(
            &self.calls,
            object(vec![
                ("method", text("create")),
                ("kwargs", Value::Object(data.clone())),
            ]),
        );
        let result = match &self.answer {
            Answer::CreateFailure(failure) => Err(failure.error()),
            Answer::NullStatus { message } => Err(SdkError::Status(Box::new(StatusError {
                status: None,
                message: message.clone(),
                headers: HeaderMap::new(),
                error_type: None,
                param: None,
                code: None,
            }))),
            Answer::Buffered { status, body } => Ok(RawCompletion::Buffered {
                status: *status,
                headers: HeaderMap::new(),
                body: body.clone().into_bytes(),
            }),
            Answer::Stream { events, failure } => Ok(RawCompletion::Stream {
                status: 200,
                headers: HeaderMap::new(),
                body: sse(events, failure.as_ref()),
            }),
        };
        ready(result)
    }
}

#[derive(Clone)]
pub(super) struct Constructor {
    pub built: Log,
    pub calls: Log,
    pub answer: Answer,
}

impl Constructor {
    pub(super) fn new(answer: Answer) -> Self {
        Self {
            built: log(),
            calls: log(),
            answer,
        }
    }
}

impl ClientConstructor for Constructor {
    type Client = FakeSdk;

    fn construct(
        &self,
        arguments: &SdkArguments,
        kind: ClientKind,
    ) -> Result<Option<FakeSdk>, SdkError> {
        push(
            &self.built,
            object(vec![
                ("kind", serde_json::to_value(kind).unwrap_or(Value::Null)),
                (
                    "http_client",
                    serde_json::to_value(arguments.http_client).unwrap_or(Value::Null),
                ),
                (
                    "api_key",
                    arguments.api_key.as_ref().map_or(Value::Null, |key| {
                        text(secrecy::ExposeSecret::expose_secret(key))
                    }),
                ),
                (
                    "base_url",
                    arguments
                        .base_url
                        .as_ref()
                        .map_or(Value::Null, |value| text(value)),
                ),
                ("timeout", Value::from(arguments.timeout)),
                (
                    "max_retries",
                    arguments.max_retries.map_or(Value::Null, Value::from),
                ),
                (
                    "organization",
                    arguments
                        .organization
                        .as_ref()
                        .map_or(Value::Null, |value| text(value)),
                ),
            ]),
        );
        Ok(Some(FakeSdk {
            kind,
            calls: Arc::clone(&self.calls),
            answer: self.answer.clone(),
        }))
    }
}

#[derive(Default)]
pub(super) struct RecordingLogger {
    pub pre: Vec<PreCallRecord>,
    pub post: Vec<PostCallRecord>,
    pub stream_options: Option<StreamOptions>,
}

impl CompletionLogger for RecordingLogger {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(record);
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post.push(record);
    }

    fn stream_defaults(&self) -> StreamDefaults {
        StreamDefaults {
            stream_options: self.stream_options.clone(),
            merge_reasoning_in_choices: false,
            is_function_call: false,
            base_hidden_params: JsonObject::new(),
        }
    }
}

pub(super) fn transport(
    answer: Answer,
) -> (
    super::super::TextCompletionTransport<Constructor>,
    Constructor,
) {
    let constructor = Constructor::new(answer);
    let copy = constructor.clone();
    (
        super::super::TextCompletionTransport {
            constructor,
            configs: super::super::TextCompletionConfigs {
                open_ai: transform_completion_request,
                fireworks_ai: transform_completion_request,
                together_ai: transform_completion_request,
                text_completion_inception: transform_completion_request,
            },
            now: fixed_instant,
            next_id: fixed_id,
        },
        copy,
    )
}

fn fixed_instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_700_000_000_i64,
        timestamp: 1_700_000_000.25_f64,
    }
}

fn fixed_id() -> Uuid {
    Uuid::nil()
}

pub(super) fn key(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn header_map(headers: &SecretHeaders) -> BTreeMap<String, String> {
    headers.0.clone()
}

pub(super) fn chunk(value: &str, finish_reason: Option<&str>) -> Value {
    object(vec![
        ("id", text("cmpl-1")),
        ("object", text("text_completion")),
        ("created", Value::from(1234_i64)),
        ("model", text("text-davinci-003")),
        (
            "choices",
            Value::Array(vec![object(vec![
                ("text", text(value)),
                ("index", Value::from(0_i64)),
                ("logprobs", Value::Null),
                ("finish_reason", finish_reason.map_or(Value::Null, text)),
            ])]),
        ),
    ])
}

pub(super) fn completion_body() -> Value {
    let mut body = chunk("hello", Some("stop"));
    body["usage"] = object(vec![
        ("prompt_tokens", Value::from(3_i64)),
        ("completion_tokens", Value::from(2_i64)),
        ("total_tokens", Value::from(5_i64)),
    ]);
    body
}

pub(super) fn message(value: &str) -> Value {
    object(vec![("role", text("user")), ("content", text(value))])
}

pub(super) struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles")
}

pub(super) fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"text-davinci-003","call_type":"text_completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the fixture is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated".to_owned(),
        Duration::default,
    )
    .expect("the fixture builds")
}

pub(super) fn prepost_env<'env>(
    state: &LoggingState,
    redactor: &'env Redactor,
    sink: &'env DiscardingSink,
) -> PrePostEnv<'env> {
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
        now: state.start_time,
    }
}
