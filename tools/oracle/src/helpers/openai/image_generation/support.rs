use std::sync::{Mutex, PoisonError};
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
use chrono::DateTime;
use serde_json::{Value, json};

use crate::helpers::HelperError;

pub(crate) const NOW: i64 = 1_700_000_000;

const INIT: &str = r#"{"model":"dall-e-2","call_type":"image_generation","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

pub(crate) fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

// the logger_fn hook runs on every pre/post call, so this keeps the record each phase saw
#[derive(Default)]
pub(crate) struct Recorder(Mutex<Vec<Value>>);

impl Recorder {
    fn record(&self, details: &Json) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(serde_json::to_value(details).unwrap_or_default());
    }

    // the two callbacks see the same input and additional_args LiteLLM hands its logging object
    pub(crate) fn last(&self, phase: &str) -> Option<Value> {
        let details = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .rev()
            .find(|details| details.get("log_event_type") == Some(&json!(phase)))?
            .clone();
        let at = |path: &str| details.pointer(path).cloned().unwrap_or_default();
        let mut header_names: Vec<&String> = details
            .pointer("/additional_args/headers")
            .and_then(Value::as_object)
            .map(|headers| headers.keys().collect())
            .unwrap_or_default();
        header_names.sort();
        let api_base = details
            .pointer("/additional_args/api_base")
            .and_then(Value::as_str)
            .map(|base| base.trim_end_matches('/'));
        Some(json!({
            "input": at("/input"),
            "additional_args": {
                "api_base": api_base,
                "complete_input_dict": at("/additional_args/complete_input_dict"),
                "acompletion": at("/additional_args/acompletion"),
                "header_names": header_names,
            },
        }))
    }
}

pub(crate) fn observe<T, A>(recorder: &Recorder, action: A) -> Result<T, Box<HelperError>>
where
    A: FnOnce(&mut LoggingState, &PrePostEnv<'_>) -> T,
{
    let redactor =
        Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| invalid(error.to_string()))?;
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).map_err(|error| invalid(error.to_string()))?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(|error| invalid(error.to_string()))?;
    let sink = Silent;
    let logger = |details: &Json| {
        recorder.record(details);
        Ok(())
    };
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: Some(&logger),
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| invalid(error.to_string()))?,
    };
    let result = action(&mut state, &env);
    Ok(result)
}
