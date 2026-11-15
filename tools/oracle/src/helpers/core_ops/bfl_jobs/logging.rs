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

use super::failure;
use crate::helpers::HelperError;

const INIT: &str = r#"{"model":"flux-pro-1.1","call_type":"image_generation","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";

pub(super) struct QuietSink;

impl LogSink for QuietSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging_state() -> Result<(Redactor, LoggingState), Box<HelperError>> {
    let init: LoggingInit = serde_json::from_str(INIT).map_err(|error| failure(&error))?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|error| failure(&error))?;
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .map_err(|error| failure(&error))?;
    Ok((redactor, state))
}

pub(super) fn logging_env<'setup>(
    redactor: &'setup Redactor,
    sink: &'setup QuietSink,
    logger: &'setup dyn Fn(&Json) -> Result<(), PrePostError>,
) -> Result<PrePostEnv<'setup>, Box<HelperError>> {
    let now = DateTime::parse_from_rfc3339(NOW).map_err(|error| failure(&error))?;
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
        now,
    })
}

pub(super) fn pre_call_effect(details: &Json) -> Value {
    let Json::Object(fields) = details else {
        return json!({"effect": "pre_call"});
    };
    let additional = match fields.get("additional_args") {
        Some(Json::Object(arguments)) => Some(arguments),
        _ => None,
    };
    json!({
        "effect": "pre_call",
        "input": fields.get("input"),
        "api_base": additional.and_then(|arguments| arguments.get("api_base")),
        "body": additional.and_then(|arguments| arguments.get("complete_input_dict")),
        "header_names": header_names(additional.and_then(|arguments| arguments.get("headers"))),
    })
}

fn header_names(headers: Option<&Json>) -> Vec<String> {
    let Some(Json::Object(headers)) = headers else {
        return Vec::new();
    };
    let mut names: Vec<String> = headers.keys().map(|name| name.to_lowercase()).collect();
    names.sort();
    names
}
