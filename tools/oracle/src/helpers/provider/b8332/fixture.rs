use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{StreamResponse, UpstreamResponse};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::huggingface_chat::mapping::HubResponse;
use betterlite_openai::huggingface_embedding::response::OrdinaryEncoder;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_provider::model_hub_dispatch::credentials::Globals;
use betterlite_provider::model_hub_dispatch::hub::headers_from;
use betterlite_provider::model_hub_dispatch::transport::{HubGetAsync, PostTransport, WireRequest};
use betterlite_provider::model_hub_dispatch::{DispatchDeps, HubDispatchError, ReplicateServices};
use betterlite_provider::oci_common::Clock;
use betterlite_provider::operation::OperationError;
use betterlite_provider::predibase::reply::TokenCounter as PredibaseTokenCounter;
use betterlite_provider::replicate::poll::ReplicateSleeper;
use betterlite_provider::replicate::poll::transport::{
    ReplicateRequest, ReplicateResponse, ReplicateTransport,
};
use betterlite_provider::replicate::reply::{
    PostCall, PostCallLogger, TokenCounter as ReplicateTokenCounter,
};
use betterlite_redact::secret::Redactor;
use betterlite_tokenizer::count::error::CountError;
use chrono::{DateTime, FixedOffset, Utc};
use http::HeaderMap;

use super::Reply;
use super::wire::helper_error;

const INIT: &str = r#"{"model":"oracle","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW_SECONDS: i64 = 1_700_000_000;

// one request as the provider would see it on the wire
pub(super) struct Wire {
    pub(super) method: String,
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Option<Vec<u8>>,
}

// answers every outbound call from the case's replies, in order, and keeps what was sent
pub(super) struct Recorder {
    replies: Mutex<VecDeque<Reply>>,
    pub(super) sent: Mutex<Vec<Wire>>,
}

impl Recorder {
    pub(super) fn keep(&self, wire: Wire) {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(wire);
    }

    fn keep_request(&self, request: &WireRequest<'_>) -> Option<Reply> {
        self.keep(Wire {
            method: request.method.to_string(),
            url: request.url.to_owned(),
            headers: request.headers.clone(),
            body: request.body.map(<[u8]>::to_vec),
        });
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }
}

#[async_trait]
impl HubGetAsync for Recorder {
    async fn get(&self, request: &WireRequest<'_>) -> Result<HubResponse, HubDispatchError> {
        let reply = self
            .keep_request(request)
            .ok_or(HubDispatchError::BodyNotObject)?;
        Ok(HubResponse {
            status_code: reply.status,
            headers: BTreeMap::new(),
            body: reply.body.into_bytes(),
        })
    }
}

#[async_trait]
impl PostTransport for Recorder {
    async fn post(&self, request: &WireRequest<'_>) -> Result<UpstreamResponse, HubDispatchError> {
        let reply = self
            .keep_request(request)
            .ok_or(HubDispatchError::BodyNotObject)?;
        Ok(UpstreamResponse {
            status: reply.status,
            headers: HeaderMap::new(),
            body: reply.body.into_bytes(),
        })
    }

    async fn post_stream(
        &self,
        request: &WireRequest<'_>,
    ) -> Result<StreamResponse, HubDispatchError> {
        self.keep_request(request);
        let unsupported = OperationError::UnsupportedOperation {
            provider: String::from("b8332"),
            operation: String::from("stream"),
        };
        Err(HubDispatchError::Operation(unsupported))
    }
}

#[async_trait]
impl ReplicateTransport for Recorder {
    async fn send(&self, request: ReplicateRequest) -> Result<ReplicateResponse, TransportError> {
        let headers = headers_from(&request.headers);
        self.keep(Wire {
            method: request.method.to_string(),
            url: request.url,
            headers,
            body: request.body,
        });
        let next = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let reply = next.ok_or(TransportError::Closed)?;
        Ok(ReplicateResponse {
            status: reply.status,
            headers: HeaderMap::new(),
            body: reply.body.into_bytes(),
        })
    }
}

struct Sleeper;

#[async_trait]
impl ReplicateSleeper for Sleeper {
    async fn sleep(&self, _seconds: f64) {}
}

struct Logger;

impl PostCallLogger for Logger {
    fn post_call(&self, _call: PostCall<'_>) {}
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(NOW_SECONDS, 0).unwrap_or_default()
    }
}

struct LengthEncoder;

impl OrdinaryEncoder for LengthEncoder {
    fn encode_ordinary(&self, text: &str) -> usize {
        text.chars().count()
    }
}

struct Counting;

#[async_trait]
impl ReplicateTokenCounter for Counting {
    async fn count_messages(&self, _model: &str, messages: &[Message]) -> Result<i64, CountError> {
        i64::try_from(messages.len()).map_err(|_overflow| CountError::Overflow)
    }

    async fn count_response_tokens(&self, _model: &str, text: &str) -> Result<i64, CountError> {
        i64::try_from(text.chars().count()).map_err(|_overflow| CountError::Overflow)
    }
}

#[async_trait]
impl PredibaseTokenCounter for Counting {
    async fn count_messages(&self, messages: &[Message]) -> Option<i64> {
        i64::try_from(messages.len()).ok()
    }

    async fn count_text(&self, text: &str) -> Option<i64> {
        i64::try_from(text.chars().count()).ok()
    }
}

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

// the services a dispatch call needs, all answering from the case
pub(super) struct Fixture {
    pub(super) recorder: Arc<Recorder>,
    sleeper: Arc<Sleeper>,
    logger: Logger,
    prompts: PromptFactory,
    counter: Counting,
    encoder: LengthEncoder,
    clock: FixedClock,
}

impl Fixture {
    pub(super) fn new(replies: &[Reply]) -> Self {
        let recorder = Recorder {
            replies: Mutex::new(replies.iter().cloned().collect()),
            sent: Mutex::new(Vec::new()),
        };
        let prompts = PromptFactory {
            cache: Mutex::new(builtin_entries()),
            hub: Hub::Unreachable,
        };
        Self {
            recorder: Arc::new(recorder),
            sleeper: Arc::new(Sleeper),
            logger: Logger,
            prompts,
            counter: Counting,
            encoder: LengthEncoder,
            clock: FixedClock,
        }
    }

    pub(super) fn deps<'deps>(
        &'deps self,
        globals: &'deps Globals<'deps>,
        env: &'deps BTreeMap<String, String>,
    ) -> DispatchDeps<'deps> {
        let replicate = ReplicateServices {
            transport: Arc::<Recorder>::clone(&self.recorder),
            sleeper: Arc::<Sleeper>::clone(&self.sleeper),
            logger: &self.logger,
        };
        DispatchDeps {
            globals,
            environment: env,
            hub: &*self.recorder,
            post: &*self.recorder,
            encoder: &self.encoder,
            replicate,
            prompts: &self.prompts,
            replicate_counter: &self.counter,
            predibase_counter: &self.counter,
            clock: &self.clock,
            default_max_tokens: 4096,
            now: self.clock.now(),
            timeout: Duration::from_secs(5),
        }
    }
}

pub(super) fn log_state(redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit =
        serde_json::from_str(INIT).map_err(|error| helper_error("LoggingInit", &error))?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(|error| helper_error("LoggingState", &error))
}

pub(super) fn log_env(redactor: &Redactor) -> PrePostEnv<'_> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::<FixedOffset>::from(
            DateTime::from_timestamp(NOW_SECONDS, 0).unwrap_or_default(),
        ),
    }
}
