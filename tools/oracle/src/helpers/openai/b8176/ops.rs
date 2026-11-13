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
use betterlite_openai::response_events::{RawResponse, ReplyBody};
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
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::helpers::HelperError;
use crate::helpers::HelperResult;

use super::input::Step;

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

fn raw_response<'input>(
    step_status: u16,
    body: &'input str,
    headers: &'input IndexMap<String, String>,
) -> RawResponse<'input> {
    RawResponse {
        status: step_status,
        headers,
        text: body,
    }
}

fn event_value(event: &Value) -> HelperResult {
    let chunk: Json =
        serde_json::from_value(event.clone()).map_err(|error| invalid_input(error.to_string()))?;
    let Json::Object(fields) = chunk else {
        return Err(invalid_input(String::from("the event must be an object")));
    };
    match transform_streaming_response(&fields) {
        Ok(decoded) => {
            let class = decoded.class().class_name();
            let body = match &decoded {
                betterlite_openai::response_events::event_decode::DecodedStreamResponse::Known {
                    record,
                    ..
                } => serde_json::to_value(record),
                betterlite_openai::response_events::event_decode::DecodedStreamResponse::Generic(
                    generic,
                ) => serde_json::to_value(generic),
                betterlite_openai::response_events::event_decode::DecodedStreamResponse::Lax {
                    fields,
                    ..
                } => serde_json::to_value(Json::Object(fields.clone())),
            }
            .map_err(|error| invalid_input(error.to_string()))?;
            Ok(json!({ "ok": true, "class": class, "event": with_nulls_dropped(body) }))
        }
        Err(_failure) => Ok(json!({ "ok": false })),
    }
}

fn logging_fixture()
-> Result<(Redactor, LoggingState, DateTime<chrono::FixedOffset>), Box<HelperError>> {
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

fn reply_step(operation: &str, status: u16, body: &str, headers: &Headers) -> HelperResult {
    let lowered = headers_of(headers);
    let reply = raw_response(status, body, &lowered);
    if operation == "create" {
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
        let Ok(transformed) = transform_response(&reply, now.timestamp(), &mut state, &env) else {
            return Ok(json!({ "ok": false }));
        };
        let (id, created_at) = match &transformed.response {
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
        };
        return Ok(json!({
            "ok": true,
            "id": id,
            "created_at": created_at,
            "headers": lowered,
        }));
    }
    let operation = if operation == "get" {
        ReplyOperation::Get
    } else {
        ReplyOperation::Cancel
    };
    let Ok(ResourceReply::Response(typed)) = transform_resource_reply(operation, &reply) else {
        return Ok(json!({ "ok": false }));
    };
    Ok(json!({
        "ok": true,
        "id": typed.response.id,
        "created_at": typed.response.created_at,
        "headers": lowered,
    }))
}

fn body_step(operation: &str, status: u16, body: &str, headers: &Headers) -> HelperResult {
    let lowered = headers_of(headers);
    let reply = raw_response(status, body, &lowered);
    let operation = if operation == "delete" {
        ReplyOperation::Delete
    } else {
        ReplyOperation::InputItems
    };
    let Ok(ResourceReply::Body(decoded)) = transform_resource_reply(operation, &reply) else {
        return Ok(json!({ "ok": false }));
    };
    let value = serde_json::to_value(*decoded).map_err(|error| invalid_input(error.to_string()))?;
    Ok(json!({ "ok": true, "reply": with_nulls_dropped(value) }))
}

pub(crate) fn execute(step: &Step) -> HelperResult {
    match step {
        Step::SupportedParams { .. } => Ok(json!(params::supported_openai_params())),
        Step::MapParams { params: given } => Ok(json!(params::map_openai_params(given))),
        Step::Sanitize { params: given } => Ok(json!(params::sanitize_request(given))),
        Step::Url {
            api_base,
            global_base,
            volcengine_base,
            ark_base,
        } => Ok(Value::String(get_complete_url(BaseSources {
            api_base: api_base.as_deref(),
            global: global_base.as_deref(),
            volcengine: volcengine_base.as_deref(),
            ark: ark_base.as_deref(),
        }))),
        Step::Headers {
            api_key,
            global_key,
            ark_key,
            volcengine_key,
            extra_headers,
        } => {
            let key = api_key
                .as_deref()
                .map(|text| SecretString::from(text.to_owned()));
            let global = global_key
                .as_deref()
                .map(|text| SecretString::from(text.to_owned()));
            let ark = ark_key
                .as_deref()
                .map(|text| SecretString::from(text.to_owned()));
            let volcengine = volcengine_key
                .as_deref()
                .map(|text| SecretString::from(text.to_owned()));
            let sources = KeySources {
                litellm_params: key.as_ref(),
                global: global.as_ref(),
                ark: ark.as_ref(),
                volcengine: volcengine.as_ref(),
            };
            match validate_environment(extra_headers.clone(), sources) {
                Ok(merged) => Ok(json!({ "ok": true, "headers": merged })),
                Err(_failure) => Ok(json!({ "ok": false })),
            }
        }
        Step::StreamEvent { event } => event_value(event),
        Step::Reply {
            operation,
            status,
            body,
            headers,
        } => reply_step(operation, *status, body, headers),
        Step::BodyReply {
            operation,
            status,
            body,
            headers,
        } => body_step(operation, *status, body, headers),
        Step::Capabilities {
            model: _model,
            stream: _stream,
        } => Ok(json!({
            "fake_stream": SHOULD_FAKE_STREAM,
            "websocket": SUPPORTS_NATIVE_WEBSOCKET,
        })),
    }
}
