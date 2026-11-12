use std::sync::{Mutex, PoisonError};
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
pub(crate) struct Recorder(Mutex<Vec<Json>>);

impl Recorder {
    fn record(&self, details: &Json) -> Result<(), PrePostError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    }

    pub(crate) fn last(&self, phase: &str) -> Option<Value> {
        let records = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        records
            .iter()
            .rev()
            .find(|details| event_type(details) == Some(phase))
            .map(callback_value)
    }
}

fn event_type(details: &Json) -> Option<&str> {
    let Json::Object(fields) = details else {
        return None;
    };
    match fields.get("log_event_type") {
        Some(Json::Text(text)) => Some(text.as_str()),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

// the two callbacks see the same input and additional_args LiteLLM hands its logging object
fn callback_value(details: &Json) -> Value {
    let Json::Object(fields) = details else {
        return Value::Null;
    };
    let input = fields.get("input").cloned().unwrap_or(Value::Null);
    let additional = fields
        .get("additional_args")
        .cloned()
        .unwrap_or_else(|| Json::Object(indexmap::IndexMap::new()));
    json!({
        "input": input,
        "additional_args": arguments_value(&additional),
    })
}

fn arguments_value(additional: &Json) -> Value {
    let Json::Object(fields) = additional else {
        return empty_arguments();
    };
    let api_base = match fields.get("api_base") {
        Some(Json::Text(text)) => json!(text.trim_end_matches('/')),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Value::Null
        }
    };
    let complete_input_dict = fields
        .get("complete_input_dict")
        .cloned()
        .unwrap_or(Value::Null);
    let acompletion = fields.get("acompletion").cloned().unwrap_or(Value::Null);
    let mut header_names: Vec<String> = match fields.get("headers") {
        Some(Json::Object(headers)) => headers.keys().cloned().collect(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Vec::new()
        }
    };
    header_names.sort();
    json!({
        "api_base": api_base,
        "complete_input_dict": complete_input_dict,
        "acompletion": acompletion,
        "header_names": header_names,
    })
}

fn empty_arguments() -> Value {
    json!({
        "api_base": Value::Null,
        "complete_input_dict": Value::Null,
        "acompletion": Value::Null,
        "header_names": Vec::<String>::new(),
    })
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
    let logger = |details: &Json| recorder.record(details);
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
