use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::agentcore_stream::adapter::header_pairs;
use betterlite_bedrock::agentcore_stream::transport::StreamHandler;
use betterlite_bedrock::bedrock_http_stream::StreamDeps;
use betterlite_bedrock::invoke_events::DecoderClock;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, LoggerFn, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::Redactor;
use chrono::DateTime;
use http::{HeaderMap, HeaderName, HeaderValue};
use uuid::Uuid;

use betterlite_oracle_helper_api::error::HelperError;

const INIT: &str = r#"{"model":"bedrock/amazon.titan","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
pub(super) const NOW: &str = "2026-01-01T00:00:00+00:00";
pub(super) const SECONDS: u64 = 1_767_225_600;
pub(super) const TIMEOUT: Duration = Duration::from_secs(600);

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

// The env a helper logs through: no callbacks or sentry, and the case's fixed clock.
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

// One handler answers both the supplied and the default slot, with fixed ids and the case's fixed time.
pub(super) fn stream_deps<'deps>(
    handler: Arc<dyn StreamHandler>,
    redactor: &'deps Redactor,
) -> StreamDeps<'deps> {
    StreamDeps {
        supplied: Some(Arc::clone(&handler)),
        default: Arc::clone(&handler),
        ids: Arc::new(Counter(Mutex::new(0))),
        clock: DecoderClock {
            next_id: || Uuid::from_u128(0x5EED_0000_u128),
            now: || i64::try_from(SECONDS).unwrap_or(0),
        },
        redactor,
        timeout: TIMEOUT,
        now: UNIX_EPOCH + Duration::from_secs(SECONDS),
    }
}

pub(super) fn failure<E: Display>(error: E) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("BedrockHttpStream"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
