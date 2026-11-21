use std::convert::Infallible;
use std::future::{Future, ready};
use std::io::Error as IoError;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
use betterlite_anthropic::anthropic_http_stream::error::{PostFailure, StatusFailure};
use betterlite_anthropic::anthropic_http_stream::transport::{
    LineChunks, StreamPost, StreamReply, StreamTransport,
};
use betterlite_anthropic::chat_call::config::{
    ChatConfig, TransformOutcome, TransformRequest, TransformResponse,
};
use betterlite_anthropic::chat_call::error::ConfigError;
use betterlite_anthropic::chat_call::transport::{BufferedReply, ChatTransport};
use betterlite_anthropic::response::error::ResponseError;
use betterlite_anthropic::response::{Logging, transform_response};
use betterlite_anthropic::stream_decode::TransportChunk;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use indexmap::IndexMap;
use thiserror::Error;
use uuid::Uuid;

pub const NOW: i64 = 1_767_225_600;
pub const TIMEOUT: Duration = Duration::from_secs(1);
const INIT: &str = r#"{"model":"claude-sonnet-4-5","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1","messages":[],"litellm_params":{},"request_logging":{}}"#;
const ENV_NOW: &str = "2026-01-01T00:00:05+00:00";

#[derive(Debug, Error)]
#[error("the transport answered with a failure")]
pub struct WireFailure;

// One endpoint answers the buffered and the streamed post and keeps what the caller sent
pub struct Wire {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub lines: Vec<String>,
    pub posts: Mutex<Vec<StreamPost>>,
}

impl Wire {
    #[must_use]
    pub fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
            lines: Vec::new(),
            posts: Mutex::new(Vec::new()),
        }
    }

    pub fn recorded(&self) -> MutexGuard<'_, Vec<StreamPost>> {
        self.posts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn failure(&self) -> Option<PostFailure<WireFailure>> {
        if (200..300).contains(&self.status) {
            return None;
        }
        Some(PostFailure::Status(Box::new(StatusFailure {
            status_code: self.status,
            body: self.body.clone(),
            exception_headers: None,
            response_headers: Some(self.headers.clone()),
        })))
    }
}

impl StreamTransport for Wire {
    type Failure = WireFailure;

    fn post(
        &self,
        post: StreamPost,
    ) -> impl Future<Output = Result<StreamReply<WireFailure>, PostFailure<WireFailure>>> + Send
    {
        self.recorded().push(post);
        if let Some(failure) = self.failure() {
            return ready(Err(failure));
        }
        let decoded: LineChunks<WireFailure> = stream::iter(
            self.lines
                .clone()
                .into_iter()
                .map(|line| Ok(TransportChunk::Text(line))),
        )
        .boxed();
        ready(Ok(StreamReply {
            status_code: self.status,
            headers: self.headers.clone(),
            lines: decoded,
        }))
    }
}

impl ChatTransport for Wire {
    fn post_buffered(
        &self,
        post: StreamPost,
    ) -> impl Future<Output = Result<BufferedReply, PostFailure<WireFailure>>> + Send {
        let reply = BufferedReply {
            status_code: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        };
        self.recorded().push(post);
        ready(self.failure().map_or(Ok(reply), Err))
    }
}

// The parent Anthropic mapper belongs to another task; this one adds only what the composition reads
pub struct Mapper;

impl ChatConfig for Mapper {
    fn transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> Result<IndexMap<String, Json>, ConfigError> {
        Ok(request.optional_params.clone())
    }

    fn async_transform_request(
        &self,
        request: &TransformRequest<'_>,
    ) -> impl Future<Output = Result<IndexMap<String, Json>, ConfigError>> + Send {
        ready(self.transform_request(request))
    }

    fn transform_outcome(
        &self,
        request: &TransformRequest<'_>,
    ) -> Result<TransformOutcome, ConfigError> {
        Ok(TransformOutcome {
            body: request.optional_params.clone(),
            headers: request.headers.clone(),
        })
    }

    fn filter_request_betas(
        &self,
        _provider: &str,
        _headers: &mut IndexMap<String, String>,
        _body: &mut IndexMap<String, Json>,
    ) -> Result<(), ConfigError> {
        Ok(())
    }

    fn transform_response<Counter>(
        &self,
        response: TransformResponse<'_, '_, '_>,
        logging: Logging<'_>,
        counter: &Counter,
    ) -> Result<(), ResponseError<Counter::Failure>>
    where
        Counter: ReasoningTokenCounter,
    {
        transform_response(
            response.envelope,
            response.raw,
            response.request,
            logging,
            counter,
        )
    }
}

#[derive(Clone, Copy)]
pub struct Tally;

impl ReasoningTokenCounter for Tally {
    type Failure = Infallible;

    // No fixture in the crate feeds a reply with reasoning content, so no test reaches this answer.
    #[cfg_attr(test, mutants::skip)]
    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct Quiet;

impl LogSink for Quiet {
    fn emit(&self, _record: LogRecord) {}
}

impl StreamLogger for Quiet {
    fn post_call(&self, _call: StreamPostCall<'_>) {}
}

impl IdSource for Quiet {
    // The cases never read a stream chunk, so the id this generates is never observed.
    #[cfg_attr(test, mutants::skip)]
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error(transparent)]
    Redaction(#[from] RedactionError),
    #[error(transparent)]
    State(#[from] LoggingStateError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

// The logging state and environment one handler call needs, built once per case
pub struct Harness {
    redactor: Redactor,
    quiet: Quiet,
    now: DateTime<FixedOffset>,
}

impl Harness {
    pub fn new() -> Result<Self, HarnessError> {
        let now = DateTime::<FixedOffset>::parse_from_rfc3339(ENV_NOW)
            .map_err(|source| serde_json::Error::io(IoError::other(source)))?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
        Ok(Self {
            redactor,
            quiet: Quiet,
            now,
        })
    }

    #[must_use]
    pub fn env(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            now: self.now,
            log: &self.quiet,
            redactor: &self.redactor,
            decision: Decision {
                content: false,
                key_info: false,
            },
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
        }
    }

    pub fn state(&self) -> Result<LoggingState, HarnessError> {
        let init: LoggingInit = serde_json::from_str(INIT)?;
        let correlation = CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        };
        LoggingState::new(
            init,
            correlation,
            &self.redactor,
            || "run".to_owned(),
            Duration::default,
        )
        .map_err(HarnessError::State)
    }

    // A leaked Quiet is the same stateless sink, so its answers match the harness's own.
    #[cfg_attr(test, mutants::skip)]
    #[must_use]
    pub const fn quiet(&self) -> &Quiet {
        &self.quiet
    }
}
