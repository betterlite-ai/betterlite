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
use betterlite_openai::api::responses_event_records::records::ResponsesTerminalEvent;
use betterlite_openai::response_events::event_decode::{
    DecodedStreamResponse, parse_terminal_event, parse_terminal_response,
    transform_streaming_response,
};
use betterlite_openai::response_events::{
    RawResponse, ReplyBody, ResponsesReply, transform_response,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use serde_json::{Value, json};

use super::operations::{auth, streams, urls};
use super::{Call, Probe};
use crate::helpers::{HelperError, HelperResult};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:05:00+00:00";

pub(super) fn run(call: &Call) -> HelperResult {
    match call {
        Call::CreatedAtConversion(probes) | Call::LaxResponseValidation(probes) => {
            replies(&probes.probes)
        }
        Call::MissingErrorCode(probes)
        | Call::UnknownGenericEvent(probes)
        | Call::KnownEventVariants(probes) => events(&probes.probes),
        Call::CompletedIncompleteFailed(probes) => terminal_responses(&probes.probes),
        Call::LastTerminalWins(probes) | Call::NoTerminalEvent(probes) => {
            terminal_events(&probes.probes)
        }
        Call::NonNativeStreamingModel(probes) => streams(&probes.probes),
        Call::ResponsesAuthHeaders(probes) => auth(&probes.probes),
        Call::ResponsesBaseUrl(probes) => Ok(urls(&probes.probes)),
    }
}

pub(super) fn invalid_input(message: String) -> Box<HelperError> {
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

fn fixture(redactor: &Redactor) -> Result<(LoggingState, PrePostEnv<'_>), Box<HelperError>> {
    let init = serde_json::from_str::<LoggingInit>(INIT)
        .map_err(|error| invalid_input(error.to_string()))?;
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(|error| invalid_input(error.to_string()))?;
    let env = PrePostEnv {
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
        now: DateTime::parse_from_rfc3339(NOW).map_err(|error| invalid_input(error.to_string()))?,
    };
    Ok((state, env))
}

fn replies(probes: &[Probe]) -> HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| invalid_input(error.to_string()))?;
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        let body = probe
            .body
            .as_deref()
            .ok_or_else(|| invalid_input(String::from("a reply probe needs body")))?;
        let status = probe
            .status
            .ok_or_else(|| invalid_input(String::from("a reply probe needs status")))?;
        let now = probe
            .now
            .ok_or_else(|| invalid_input(String::from("a reply probe needs now")))?;
        let given_headers = probe.headers.clone().unwrap_or_default();
        let headers: indexmap::IndexMap<String, String> = given_headers
            .into_iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value))
            .collect();
        let (mut state, env) = fixture(&redactor)?;
        let reply = RawResponse {
            status,
            headers: &headers,
            text: body,
        };
        match transform_response(&reply, now, &mut state, &env) {
            Ok(transformed) => results.push(reply_value(&transformed, &state)),
            Err(_failure) => results.push(json!({ "error": status })),
        }
    }
    Ok(Value::Array(results))
}

fn reply_value(transformed: &ResponsesReply, state: &LoggingState) -> Value {
    let (id, created_at) = match &transformed.response {
        ReplyBody::Typed(response) => (Value::String(response.id.clone()), response.created_at),
        ReplyBody::Lax(fields) => (
            fields.get("id").map_or(Value::Null, |given| {
                serde_json::to_value(given).unwrap_or_default()
            }),
            fields
                .get("created_at")
                .and_then(|given| serde_json::to_value(given).ok())
                .and_then(|given| given.as_i64())
                .unwrap_or_default(),
        ),
    };
    let details = &state.model_call_details;
    let recorded = |name: &str| {
        details.get(name).map_or(Value::Null, |given| {
            serde_json::to_value(given).unwrap_or_default()
        })
    };
    json!({
        "id": id,
        "created_at": created_at,
        "headers": transformed.headers,
        "additional_headers": transformed.additional_headers,
        "post_call": {
            "original_response": recorded("original_response"),
            "additional_args": recorded("additional_args"),
        },
    })
}

fn events(probes: &[Probe]) -> HelperResult {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        let event = probe
            .event
            .clone()
            .ok_or_else(|| invalid_input(String::from("an event probe needs event")))?;
        let decoded = decode_event(&event)?;
        results.push(event_value(&decoded));
    }
    Ok(Value::Array(results))
}

fn decode_event(event: &Value) -> Result<DecodedStreamResponse, Box<HelperError>> {
    let chunk = serde_json::from_value::<Json>(event.clone())
        .map_err(|error| invalid_input(error.to_string()))?;
    let Json::Object(fields) = chunk else {
        return Err(invalid_input(String::from("the event must be an object")));
    };
    Ok(transform_streaming_response(&fields))
}

fn event_value(decoded: &DecodedStreamResponse) -> Value {
    let event = match decoded {
        DecodedStreamResponse::Known { record, .. } => {
            serde_json::to_value(record).unwrap_or_default()
        }
        DecodedStreamResponse::Generic(event) => serde_json::to_value(event).unwrap_or_default(),
        DecodedStreamResponse::Lax { fields, .. } => {
            serde_json::to_value(Json::Object(fields.clone())).unwrap_or_default()
        }
    };
    json!({ "class": decoded.class().class_name(), "event": strip_nulls(event) })
}

fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .filter(|(_name, field)| !field.is_null())
                .map(|(name, field)| (name, strip_nulls(field)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(strip_nulls).collect()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value,
    }
}

fn terminal_responses(probes: &[Probe]) -> HelperResult {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        let chunks = probe
            .chunks
            .as_deref()
            .ok_or_else(|| invalid_input(String::from("a terminal probe needs chunks")))?;
        results.push(parse_terminal_response(chunks).map_or(
            Value::Null,
            |response| json!({ "id": response.id, "created_at": response.created_at }),
        ));
    }
    Ok(Value::Array(results))
}

fn terminal_events(probes: &[Probe]) -> HelperResult {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        let chunks = probe
            .chunks
            .as_deref()
            .ok_or_else(|| invalid_input(String::from("a terminal probe needs chunks")))?;
        results
            .push(parse_terminal_event(chunks).map_or(Value::Null, |event| terminal_value(&event)));
    }
    Ok(Value::Array(results))
}

fn terminal_value(event: &ResponsesTerminalEvent) -> Value {
    let (class, response) = match event {
        ResponsesTerminalEvent::Completed(response) => ("ResponseCompletedEvent", response),
        ResponsesTerminalEvent::Incomplete(response) => ("ResponseIncompleteEvent", response),
        ResponsesTerminalEvent::Failed(response) => ("ResponseFailedEvent", response),
    };
    json!({
        "class": class,
        "id": response.response.id,
        "created_at": response.response.created_at,
    })
}
