use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
use betterlite_anthropic::anthropic_http_stream::error::{PostFailure, StatusFailure};
use betterlite_anthropic::anthropic_http_stream::transport::{
    LineChunks, StreamPost, StreamReply, StreamTransport,
};
use betterlite_anthropic::chat_call::config::{ChatConfig, TransformRequest, TransformResponse};
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
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use indexmap::IndexMap;
use thiserror::Error;
use uuid::Uuid;

pub(crate) const NOW: i64 = 1_767_225_600;
pub(crate) const TIMEOUT: Duration = Duration::from_secs(1);
pub(crate) const MAX_BYTES: usize = 0x0010_0000;
const INIT: &str = r#"{"model":"claude-sonnet-4-5","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1","messages":[],"litellm_params":{},"request_logging":{}}"#;
const ENV_NOW: &str = "2026-01-01T00:00:05+00:00";

#[derive(Debug, Error)]
#[error("the transport answered with a failure")]
pub(crate) struct WireFailure;

// One endpoint answers both the buffered and the streamed post and keeps what the caller sent
pub(crate) struct Wire {
    pub(crate) status: u16,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
    pub(crate) lines: Vec<String>,
    pub(crate) posts: Mutex<Vec<StreamPost>>,
}

impl Wire {
    pub(crate) fn recorded(&self) -> MutexGuard<'_, Vec<StreamPost>> {
        self.posts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn buffered_reply(&self) -> BufferedReply {
        BufferedReply {
            status_code: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        }
    }

    fn begin(&self, post: StreamPost) -> Option<PostFailure<WireFailure>> {
        self.recorded().push(post);
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
        if let Some(failure) = self.begin(post) {
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
        ready(
            self.begin(post)
                .map_or_else(|| Ok(self.buffered_reply()), Err),
        )
    }
}

// The parent Anthropic mapper is B584's; this stands in for it and answers with the promoted options as the body
pub(crate) struct Mapper;

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
pub(crate) struct Tally;

impl ReasoningTokenCounter for Tally {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Infallible> {
        Ok(0)
    }
}

pub(crate) struct Quiet;

impl LogSink for Quiet {
    fn emit(&self, _record: LogRecord) {}
}

impl StreamLogger for Quiet {
    fn post_call(&self, _call: StreamPostCall<'_>) {}
}

impl IdSource for Quiet {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

pub(crate) struct Runtime {
    redactor: Redactor,
    sink: Quiet,
    now: DateTime<FixedOffset>,
}

impl Runtime {
    pub(crate) fn new() -> Result<Self, Box<HelperError>> {
        Ok(Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?,
            sink: Quiet,
            now: DateTime::<FixedOffset>::parse_from_rfc3339(ENV_NOW).or_value_error()?,
        })
    }

    pub(crate) fn env(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            now: self.now,
            log: &self.sink,
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

    pub(crate) fn state(&self) -> Result<LoggingState, Box<HelperError>> {
        let init: LoggingInit = serde_json::from_str(INIT).or_value_error()?;
        LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &self.redactor,
            || "run".to_owned(),
            Duration::default,
        )
        .or_value_error()
    }
}
