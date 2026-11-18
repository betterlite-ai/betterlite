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
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::super::output::AdapterError;

pub(crate) struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) fn redactor() -> Result<Redactor, AdapterError> {
    Ok(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?)
}

pub(crate) fn state(redactor: &Redactor) -> Result<LoggingState, AdapterError> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"amazon.titan-embed-text-v1","call_type":"embedding","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )?;
    Ok(LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )?)
}

pub(crate) fn env<'env>(
    state: &LoggingState,
    redactor: &'env Redactor,
    sink: &'env DiscardingSink,
    capture: &'env dyn Fn(&Json) -> Result<(), PrePostError>,
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
        logger_fn: Some(capture),
        now: state.start_time,
    }
}

#[derive(Clone, Default)]
pub(crate) struct Capture(Arc<Mutex<Vec<Json>>>);

impl Capture {
    pub(crate) fn push(&self, details: &Json) -> Result<(), PrePostError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    }

    pub(crate) fn phases(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter_map(|snapshot| match snapshot {
                Json::Object(fields) => match fields.get("log_event_type") {
                    Some(Json::Text(phase)) => Some(phase.clone()),
                    Some(_) | None => None,
                },
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    None
                }
            })
            .collect()
    }
}
