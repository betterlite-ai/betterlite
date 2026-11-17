use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::agentcore_stream::adapter::header_pairs;
use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use http::{HeaderMap, HeaderName, HeaderValue};
use uuid::Uuid;

use betterlite_oracle_helper_api::error::HelperError;

const INIT: &str = r#"{"model":"bedrock/amazon.titan","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:00+00:00";
pub(super) const SECONDS: u64 = 1_767_225_600;
pub(super) const TIMEOUT: Duration = Duration::from_secs(600);

pub(super) struct Quiet;

impl LogSink for Quiet {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct Counter(AtomicUsize);

impl Counter {
    pub(super) fn new() -> Self {
        Self(AtomicUsize::new(0))
    }
}

impl IdSource for Counter {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(u128::from(self.0.fetch_add(1, Ordering::Relaxed)))
    }
}

pub(super) fn fixed_id() -> Uuid {
    Uuid::from_u128(0x5EED_0000_u128)
}

pub(super) fn fixed_seconds() -> i64 {
    i64::try_from(SECONDS).unwrap_or(0)
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

pub(super) fn env_time() -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    DateTime::parse_from_rfc3339(NOW).map_err(failure)
}

pub(super) fn header_map(pairs: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(failure)?;
        let value = HeaderValue::from_str(value).map_err(failure)?;
        headers.append(name, value);
    }
    Ok(headers)
}

pub(super) fn sorted_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    let mut pairs = header_pairs(headers);
    pairs.sort();
    pairs
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
