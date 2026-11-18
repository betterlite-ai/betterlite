use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::agentcore_stream::adapter::header_pairs;
use betterlite_bedrock::agentcore_stream::transport::StreamHandler;
use betterlite_bedrock::bedrock_http_stream::StreamDeps;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::invoke_events::DecoderClock;
use betterlite_bedrock::invoke_stream::timing::{ApiTiming, TimingCall};
use betterlite_bedrock::invoke_stream::{
    StreamGlobals, StreamSetup, StreamTransport, WrappedInvokeStream, custom_stream_wrapper,
};
use betterlite_core::rules::{RuleDecision, RuleEvaluator};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, LoggerFn, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::logging_streams::assembly::StreamBuild;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_setup::correlation::CorrelationRestore;
use betterlite_sdk::chat_stream::config::REPEATED_CHUNK_LIMIT;
use betterlite_sdk::chat_stream::ports::{
    ChatStreamPorts, StreamClock, StreamFailure, StreamLogging,
};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use chrono::DateTime;
use http::{HeaderMap, HeaderName, HeaderValue};
use tokio::runtime::Runtime;
use uuid::Uuid;

use betterlite_oracle_helper_api::error::HelperError;

const INIT: &str = r#"{"model":"bedrock/amazon.titan","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:00+00:00";
const SECONDS: u64 = 1_767_225_600;
const TIMEOUT: Duration = Duration::from_secs(600);

pub(super) struct Quiet;

impl LogSink for Quiet {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct Counter(Mutex<u128>);

impl IdSource for Counter {
    fn next_id(&self) -> Uuid {
        let mut next = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let id = Uuid::from_u128(*next);
        *next = next.saturating_add(1);
        id
    }
}

pub(super) fn logging_state(redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).map_err(failure)?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(failure)
}

pub(super) fn pre_post_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Quiet,
    logger: LoggerFn<'env>,
) -> Result<PrePostEnv<'env>, Box<HelperError>> {
    Ok(PrePostEnv {
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
        logger_fn: Some(logger),
        now: DateTime::parse_from_rfc3339(NOW).map_err(failure)?,
    })
}

// The ports' state the identity case has to own: a request map and the correlation the wrapper restores.
#[derive(Default)]
pub(super) struct World {
    request_data: RequestMap,
    correlation: CorrelationContext,
}

// B004's ports read here; the identity payload names none of them, so each one answers a fixed value.
pub(super) struct Clock;

impl StreamClock for Clock {
    fn now(&self) -> StreamInstant {
        StreamInstant {
            seconds: i64::try_from(SECONDS).unwrap_or(0),
            timestamp: 0.0,
        }
    }
}

pub(super) struct QuietLogging;

impl StreamLogging for QuietLogging {
    fn started(&self, _at: StreamInstant) {}

    fn log_chunk(&self, _chunk: &StreamChunk, _cache_hit: bool) {}

    fn completed(&self, _chunks: &[StreamChunk]) -> StreamBuild {
        StreamBuild::Empty
    }

    fn partial_usage(&self, _chunks: &[StreamChunk]) {}

    fn log_failure(&self, _failure: &StreamFailure) {}

    fn log_success(&self, _response: Option<Json>, _cache_hit: bool) {}
}

pub(super) struct NoTiming;

impl ApiTiming for NoTiming {
    fn service_success(&self, _call: &TimingCall<'_>) {}
}

pub(super) struct NoRules;

impl RuleEvaluator for NoRules {
    fn evaluate(&self, _index: usize, _input: &str) -> Option<RuleDecision> {
        None
    }
}

pub(super) struct NoRestore;

impl CorrelationRestore for NoRestore {
    fn restore_correlation_context(&self, _context: &mut CorrelationContext) {}
}

// The wrapper B004 builds over B691's opened stream; the identity payload reads it without consuming the body, as LiteLLM's own case does.
pub(super) fn wrapper_of<'deps>(
    runtime: &Runtime,
    transport: &StreamTransport<'_>,
    deps: &StreamDeps<'deps>,
    world: &'deps mut World,
    log: &mut LogContext<'_, '_>,
) -> Result<WrappedInvokeStream<'deps>, Box<HelperError>> {
    let World {
        request_data,
        correlation,
    } = world;
    let ports = ChatStreamPorts {
        clock: &Clock,
        logging: &QuietLogging,
        cache: None,
        hooks: &[],
        request_data,
        call_type: "completion",
        rules: &NoRules,
        restore: &NoRestore,
        correlation,
        redactor: deps.redactor,
    };
    let setup = StreamSetup {
        ports,
        timing: &NoTiming,
        globals: StreamGlobals {
            max_streaming_duration: None,
            repeated_chunk_limit: REPEATED_CHUNK_LIMIT,
            logging_disable: false,
        },
    };
    runtime
        .block_on(custom_stream_wrapper(transport, deps, setup, log))
        .map_err(failure)
}

// The supplied slot is the case's caller client; the default one stands for the module's own client.
pub(super) fn stream_deps(
    supplied: Option<Arc<dyn StreamHandler>>,
    default: Arc<dyn StreamHandler>,
    redactor: &Redactor,
) -> StreamDeps<'_> {
    StreamDeps {
        supplied,
        default,
        ids: Arc::new(Counter(Mutex::new(0))),
        clock: DecoderClock {
            next_id: || Uuid::from_u128(0x5EED_0000_u128),
            now: || i64::try_from(SECONDS).unwrap_or(0),
        },
        redactor,
        timeout: TIMEOUT,
        now: now(),
    }
}

pub(super) fn header_map(pairs: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (raw_name, raw_value) in pairs {
        let name = HeaderName::from_bytes(raw_name.as_bytes()).map_err(failure)?;
        let value = HeaderValue::from_str(raw_value).map_err(failure)?;
        headers.append(name, value);
    }
    Ok(headers)
}

pub(super) fn sorted_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    let mut pairs = header_pairs(headers);
    pairs.sort();
    pairs
}

pub(super) fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: String::from("BedrockInvokeStream"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(SECONDS))
        .unwrap_or(UNIX_EPOCH)
}
