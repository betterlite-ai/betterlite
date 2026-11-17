use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use http::HeaderMap;
use indexmap::IndexMap;
use thiserror::Error;
use uuid::Uuid;

use crate::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
use crate::anthropic_http_stream::error::{PostFailure, StatusFailure};
use crate::anthropic_http_stream::transport::{
    LineChunks, StreamPost, StreamReply, StreamTransport,
};
use crate::chat_call::ChatCallError;
use crate::chat_call::config::{ChatConfig, TransformRequest};
use crate::chat_call::error::ConfigError;
use crate::chat_call::transport::{BufferedReply, ChatTransport};
use crate::response::error::ResponseError;
use crate::response::wire::ResponseEnvelope;
use crate::response::{
    Logging as ResponseLogging, RawResponse, ResponseRequest, transform_response,
};
use crate::stream_decode::TransportChunk;
use crate::stream_state::state::IdSource;
use crate::usage::ReasoningTokenCounter;

pub(super) const MODEL: &str = "claude-sonnet-4-6";
pub(super) const API_BASE: &str = "https://api.anthropic.com/v1/messages";
pub(super) const CALLER_KEY: &str = "sk-ant-caller-0123456789";
pub(super) const SETTINGS_KEY: &str = "sk-ant-settings-0123456789";
pub(super) const NOW: i64 = 1_767_225_600;
pub(super) const TIMEOUT: Duration = Duration::from_secs(1);

const INIT: &str = r#"{"model":"claude-sonnet-4-6","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1","messages":[],"litellm_params":{},"request_logging":{}}"#;
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
    // httpx raises litellm.Timeout, which carries status 408, for a deadline that never answered
    Timed,
    Status {
        status_code: u16,
        headers: HeaderMap,
        body: Vec<u8>,
        // make_call prefers exception.headers; httpx masks a status error and leaves that attribute absent
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
        let body = text_body(content);
        Self {
            status_code: 200,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
            lines: vec![
                "event: message_start".to_owned(),
                "data: {\"type\":\"message_start\"}".to_owned(),
            ],
        }
    }

    pub(super) fn raw(status_code: u16, headers: HeaderMap, body: &str) -> Self {
        Self {
            status_code,
            headers,
            body: body.as_bytes().to_vec(),
            lines: body.lines().map(str::to_owned).collect(),
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
    pub(super) posts: Mutex<Vec<StreamPost>>,
}

impl FixtureTransport {
    pub(super) fn new(reply: FixtureReply, trip: Trip) -> Self {
        Self {
            reply,
            trip,
            posts: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, post: StreamPost) {
        self.posts.lock().unwrap().push(post);
    }

    fn failure(&self) -> Option<PostFailure<FixtureFailure>> {
        match &self.trip {
            Trip::Reply => None,
            Trip::Unknown(message) => Some(PostFailure::Unknown(FixtureFailure {
                message: (*message).to_owned(),
            })),
            Trip::Timed => Some(PostFailure::Unknown(FixtureFailure {
                message: "timed out after 1.5 seconds".to_owned(),
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

    pub(super) fn posted_bodies(&self) -> Vec<IndexMap<String, Json>> {
        self.posts
            .lock()
            .unwrap()
            .iter()
            .map(|post| serde_json::from_slice(&post.body).unwrap())
            .collect()
    }

    pub(super) fn posted_headers(&self) -> Vec<HeaderMap> {
        self.posts
            .lock()
            .unwrap()
            .iter()
            .map(|post| post.headers.clone())
            .collect()
    }

    pub(super) fn posts(&self) -> usize {
        self.posts.lock().unwrap().len()
    }
}

impl StreamTransport for FixtureTransport {
    type Failure = FixtureFailure;

    async fn post(
        &self,
        post: StreamPost,
    ) -> Result<StreamReply<FixtureFailure>, PostFailure<FixtureFailure>> {
        self.record(post);
        if let Some(failure) = self.failure() {
            return Err(failure);
        }
        let lines: Vec<String> = self.reply.lines.clone();
        let decoded: LineChunks<FixtureFailure> = futures_util::stream::iter(
            lines.into_iter().map(|line| Ok(TransportChunk::Text(line))),
        )
        .boxed();
        Ok(StreamReply {
            status_code: self.reply.status_code,
            headers: self.reply.headers.clone(),
            lines: decoded,
        })
    }
}

impl ChatTransport for FixtureTransport {
    async fn post_buffered(
        &self,
        post: StreamPost,
    ) -> Result<BufferedReply, PostFailure<FixtureFailure>> {
        self.record(post);
        match self.failure() {
            Some(failure) => Err(failure),
            None => Ok(BufferedReply {
                status_code: self.reply.status_code,
                headers: self.reply.headers.clone(),
                body: self.reply.body.clone(),
            }),
        }
    }
}

pub(super) struct Recorded {
    pub(super) model: String,
    pub(super) messages: Vec<Json>,
    pub(super) optional_params: IndexMap<String, Json>,
    pub(super) litellm_params: IndexMap<String, Json>,
    pub(super) headers: IndexMap<String, String>,
}

pub(super) struct FixtureConfig {
    pub(super) body: IndexMap<String, Json>,
    pub(super) transform_failure: Option<String>,
    pub(super) filter_failure: Option<String>,
    pub(super) transforms: Mutex<Vec<Recorded>>,
    pub(super) calls: Mutex<Vec<String>>,
}

impl FixtureConfig {
    fn record(
        &self,
        request: &TransformRequest<'_>,
        mode: &str,
    ) -> Result<IndexMap<String, Json>, ConfigError> {
        self.calls.lock().unwrap().push(mode.to_owned());
        self.transforms.lock().unwrap().push(Recorded {
            model: request.model.to_owned(),
            messages: request.messages.to_vec(),
            optional_params: request.optional_params.clone(),
            litellm_params: request.litellm_params.clone(),
            headers: request.headers.values.clone(),
        });
        match &self.transform_failure {
            Some(message) => Err(Box::new(FixtureFailure {
                message: message.clone(),
            })),
            None => Ok(self.body.clone()),
        }
    }
}

impl ChatConfig for FixtureConfig {
    fn transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> Result<IndexMap<String, Json>, ConfigError> {
        self.record(request, "sync")
    }

    fn async_transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> impl Future<Output = Result<IndexMap<String, Json>, ConfigError>> + Send {
        let recorded = self.record(request, "async");
        ready(recorded)
    }

    fn filter_request_betas(
        &self,
        provider: &str,
        headers: &mut IndexMap<String, String>,
        body: &mut IndexMap<String, Json>,
    ) -> Result<(), ConfigError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("filter:{provider}"));
        if let Some(message) = &self.filter_failure {
            return Err(Box::new(FixtureFailure {
                message: message.clone(),
            }));
        }
        if let Some(value) = headers.get_mut("anthropic-beta") {
            *value = "filtered".to_owned();
        }
        if body.contains_key("anthropic_beta") {
            body.insert(
                "anthropic_beta".to_owned(),
                Json::List(vec![Json::Text("kept".to_owned())]),
            );
        }
        Ok(())
    }

    fn transform_response<Counter>(
        &self,
        envelope: &mut ResponseEnvelope,
        raw: &RawResponse<'_>,
        request: &ResponseRequest<'_>,
        logging: ResponseLogging<'_>,
        counter: &Counter,
    ) -> Result<(), ResponseError<Counter::Failure>>
    where
        Counter: ReasoningTokenCounter,
    {
        transform_response(envelope, raw, request, logging, counter)
    }
}

#[derive(Default)]
pub(super) struct RecordingLogger {
    pub(super) calls: Mutex<Vec<StreamLoggerCall>>,
}

pub(super) struct StreamLoggerCall {
    pub(super) original_response: String,
    pub(super) complete_input_dict: String,
}

impl StreamLogger for RecordingLogger {
    fn post_call(&self, call: StreamPostCall<'_>) {
        self.calls.lock().unwrap().push(StreamLoggerCall {
            original_response: call.original_response.to_owned(),
            complete_input_dict: call.complete_input_dict.to_owned(),
        });
    }
}

#[derive(Clone)]
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

pub(super) type CallError = ChatCallError<FixtureFailure, Infallible>;
