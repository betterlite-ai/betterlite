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
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::response_events::event_decode::DecodedStreamResponse;
use betterlite_openai::response_events::{RawResponse, ReplyBody, ResponsesReply};
use betterlite_openai::volcengine_responses::reply::{
    ReplyOperation, ResourceReply, transform_resource_reply, transform_response,
};
use betterlite_openai::volcengine_responses::request::{
    BaseSources, KeySources, get_complete_url, validate_environment,
};
use betterlite_openai::volcengine_responses::stream::transform_streaming_response;
use betterlite_openai::volcengine_responses::{
    SHOULD_FAKE_STREAM, SUPPORTS_NATIVE_WEBSOCKET, params,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::HelperError;
use crate::helpers::HelperResult;

use super::input::{HeadersStep, ReplyStep, Step, StreamEvent, Url};

const INIT: &str = r#"{"model":"volcengine-model","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:05:00+00:00";

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn with_nulls_dropped(value: Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(name, field)| (name, with_nulls_dropped(field)))
                .filter(|(_name, field)| !field.is_null())
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(with_nulls_dropped).collect()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value,
    }
}

fn headers_of(given: &Headers) -> IndexMap<String, String> {
    given
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect()
}

fn event_value(step: &StreamEvent) -> HelperResult {
    let chunk: Json = serde_json::from_value(step.event.clone())
        .map_err(|error| invalid_input(error.to_string()))?;
    let Json::Object(fields) = chunk else {
        return Err(invalid_input(String::from("the event must be an object")));
    };
    match transform_streaming_response(&fields) {
        Ok(decoded) => {
            let class = decoded.class().class_name();
            let body = match &decoded {
                DecodedStreamResponse::Known { record, .. } => serde_json::to_value(record),
                DecodedStreamResponse::Generic(generic) => serde_json::to_value(generic),
                DecodedStreamResponse::Lax { fields, .. } => {
                    serde_json::to_value(Json::Object(fields.clone()))
                }
            }
            .map_err(|error| invalid_input(error.to_string()))?;
            Ok(json!({ "ok": true, "class": class, "event": with_nulls_dropped(body) }))
        }
        Err(_failure) => Ok(json!({ "ok": false })),
    }
}

fn logging_fixture() -> Result<(Redactor, LoggingState, DateTime<FixedOffset>), Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| invalid_input(error.to_string()))?;
    let init = serde_json::from_str::<LoggingInit>(INIT)
        .map_err(|error| invalid_input(error.to_string()))?;
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(|error| invalid_input(error.to_string()))?;
    let now =
        DateTime::parse_from_rfc3339(NOW).map_err(|error| invalid_input(error.to_string()))?;
    Ok((redactor, state, now))
}

fn reply_identity(reply: &ResponsesReply) -> (Value, i64) {
    match &reply.response {
        ReplyBody::Typed(response) => (Value::String(response.id.clone()), response.created_at),
        ReplyBody::Lax(fields) => (
            fields
                .get("id")
                .and_then(|given| serde_json::to_value(given).ok())
                .unwrap_or(Value::Null),
            fields
                .get("created_at")
                .and_then(|given| serde_json::to_value(given).ok())
                .and_then(|given| given.as_i64())
                .unwrap_or_default(),
        ),
    }
}

fn create_reply(reply: &RawResponse<'_>, headers: &IndexMap<String, String>) -> HelperResult {
    let (redactor, mut state, now) = logging_fixture()?;
    let silent = Silent;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    };
    let Ok(transformed) = transform_response(reply, now.timestamp(), &mut state, &env) else {
        return Ok(json!({ "ok": false }));
    };
    let (id, created_at) = reply_identity(&transformed);
    Ok(json!({ "ok": true, "id": id, "created_at": created_at, "headers": headers }))
}

fn resource_reply(
    operation: ReplyOperation,
    reply: &RawResponse<'_>,
    headers: &IndexMap<String, String>,
) -> Value {
    let Ok(ResourceReply::Response(typed)) = transform_resource_reply(operation, reply) else {
        return json!({ "ok": false });
    };
    json!({
        "ok": true,
        "id": typed.response.id,
        "created_at": typed.response.created_at,
        "headers": headers,
    })
}

fn reply_step(step: &ReplyStep) -> HelperResult {
    let lowered = headers_of(&step.headers);
    let reply = RawResponse {
        status: step.status,
        headers: &lowered,
        text: &step.body,
    };
    if step.operation == "create" {
        return create_reply(&reply, &lowered);
    }
    let operation = if step.operation == "get" {
        ReplyOperation::Get
    } else {
        ReplyOperation::Cancel
    };
    Ok(resource_reply(operation, &reply, &lowered))
}

fn url_step(step: &Url) -> String {
    get_complete_url(BaseSources {
        api_base: step.api.as_deref(),
        global: step.global.as_deref(),
        volcengine: step.volcengine.as_deref(),
        ark: step.ark.as_deref(),
    })
}

fn headers_step(step: &HeadersStep) -> Value {
    let params_key = step.api.as_deref().map(SecretString::from);
    let global_key = step.global.as_deref().map(SecretString::from);
    let ark_key = step.ark.as_deref().map(SecretString::from);
    let volcengine_key = step.volcengine.as_deref().map(SecretString::from);
    let sources = KeySources {
        litellm_params: params_key.as_ref(),
        global: global_key.as_ref(),
        ark: ark_key.as_ref(),
        volcengine: volcengine_key.as_ref(),
    };
    match validate_environment(&step.extra_headers, sources) {
        Ok(merged) => json!({ "ok": true, "headers": merged }),
        Err(_failure) => json!({ "ok": false }),
    }
}

pub(crate) fn execute(step: &Step) -> HelperResult {
    match step {
        Step::SupportedParams(_) => Ok(json!(params::supported_openai_params())),
        Step::Sanitize(given) => Ok(json!(params::sanitize_request(&given.params))),
        Step::Url(given) => Ok(Value::String(url_step(given))),
        Step::Headers(given) => Ok(headers_step(given)),
        Step::StreamEvent(given) => event_value(given),
        Step::Reply(given) => reply_step(given),
        Step::Capabilities(_) => Ok(json!({
            "fake_stream": SHOULD_FAKE_STREAM,
            "websocket": SUPPORTS_NATIVE_WEBSOCKET,
        })),
    }
}
