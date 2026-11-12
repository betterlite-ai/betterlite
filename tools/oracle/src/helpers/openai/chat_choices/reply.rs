use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::reply::{ResponseReply, transform_response};
use betterlite_openai::compatible_wire::LogInput;
use betterlite_openai::openai_base_config::Headers;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, to_value};

use super::endpoint::value;
use crate::helpers::{HelperError, HelperResult};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseSteps {
    steps: Vec<ResponseStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseStep {
    status: u16,
    raw_text: String,
    #[serde(default)]
    raw_headers: Headers,
    #[serde(default)]
    messages: Option<Json>,
    #[serde(default)]
    request_data: IndexMap<String, Json>,
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn run(steps: &ResponseSteps) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
    let mut outputs = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        outputs.push(one(step, &redactor)?);
    }
    Ok(Value::Array(outputs))
}

fn one(step: &ResponseStep, redactor: &Redactor) -> Result<Value, Box<HelperError>> {
    let env = environment(redactor)?;
    let mut state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).map_err(failure)?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(failure)?;
    let convert = Mutex::new(None);
    let record = &convert;
    let result = transform_response(
        &ResponseReply {
            status: step.status,
            headers: &step.raw_headers,
            body: step.raw_text.as_bytes(),
        },
        &mut state,
        &env,
        LogInput {
            messages: step.messages.clone().unwrap_or(Json::Null),
            data: RawBody::Object(step.request_data.clone()),
        },
        |decoded, response_headers, hidden_headers| {
            *record.lock().unwrap_or_else(PoisonError::into_inner) = Some(value(&[
                ("name", Value::String(String::from("convert"))),
                ("response", decoded),
                (
                    "response_headers",
                    to_value(response_headers).unwrap_or(Value::Null),
                ),
                (
                    "hidden_headers",
                    to_value(hidden_headers).unwrap_or(Value::Null),
                ),
            ]));
            Ok(Value::String(String::from("projected")))
        },
    );
    let mut events = vec![post_call_event(&state)];
    if let Some(event) = convert.into_inner().unwrap_or_else(PoisonError::into_inner) {
        events.push(event);
    }
    let outcome = match result {
        Ok(_projected) => value(&[("kind", Value::String(String::from("reply")))]),
        Err(error) => error_outcome(&error),
    };
    Ok(value(&[
        ("events", Value::Array(events)),
        ("outcome", outcome),
    ]))
}

fn post_call_event(state: &LoggingState) -> Value {
    let details = &state.model_call_details;
    let input = details.get("input").cloned().unwrap_or(Json::Null);
    let response = details
        .get("original_response")
        .cloned()
        .unwrap_or(Json::Null);
    let data = details
        .get("additional_args")
        .and_then(|args| match args {
            Json::Object(fields) => fields.get("complete_input_dict").cloned(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .unwrap_or(Json::Null);
    value(&[
        ("name", Value::String(String::from("post_call"))),
        ("input", to_value(input).unwrap_or(Value::Null)),
        ("original_response", to_value(response).unwrap_or(Value::Null)),
        ("complete_input_dict", to_value(data).unwrap_or(Value::Null)),
    ])
}

fn error_outcome(error: &betterlite_openai::chat_choices::reply::TransformResponseError) -> Value {
    match error {
        betterlite_openai::chat_choices::reply::TransformResponseError::Json(record) => value(&[
            ("kind", Value::String(String::from("error"))),
            ("status_code", Value::from(record.status_code)),
            (
                "headers",
                record
                    .headers
                    .as_ref()
                    .map_or(Value::Null, |headers| {
                        to_value(headers).unwrap_or(Value::Null)
                    }),
            ),
        ]),
        _ => value(&[("kind", Value::String(String::from("error")))]),
    }
}

fn environment(redactor: &Redactor) -> Result<PrePostEnv<'_>, Box<HelperError>> {
    Ok(PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::<FixedOffset>::parse_from_rfc3339(NOW).map_err(failure)?,
    })
}

fn failure(error: impl std::fmt::Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
