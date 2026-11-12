#![cfg(test)]

use std::str;
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
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::Value;

use super::support::{text, value};
use crate::chat_choices::reply::{
    ResponseReply, TransformResponseError, error_class, transform_response,
};
use crate::compatible_wire::LogInput;
use crate::openai_base_config::Headers;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn logging(redactor: &Redactor) -> LoggingState {
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap()
}

fn environment(redactor: &Redactor) -> PrePostEnv<'_> {
    PrePostEnv {
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
        now: DateTime::parse_from_rfc3339("2026-01-01T00:05:00+00:00").unwrap(),
    }
}

fn input() -> LogInput {
    LogInput {
        messages: Json::List(vec![Json::Object(IndexMap::from([(
            String::from("role"),
            Json::Text(String::from("user")),
        )]))]),
        data: RawBody::Object(IndexMap::from([(
            String::from("model"),
            Json::Text(String::from("gpt-4o")),
        )])),
    }
}

fn headers() -> Headers {
    Headers::from([
        (
            String::from("content-type"),
            String::from("application/json"),
        ),
        (String::from("x-request-id"), String::from("req-1")),
    ])
}

fn headers_value() -> Value {
    value(&[
        ("content-type", text("application/json")),
        ("x-request-id", text("req-1")),
    ])
}

fn respond(
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    status: u16,
    body: &[u8],
) -> Result<(Value, Value), (TransformResponseError, Value)> {
    let wire_headers = headers();
    let events = Mutex::new(Vec::new());
    let record = &events;
    let result = transform_response(
        &ResponseReply {
            status,
            headers: &wire_headers,
            body,
        },
        state,
        env,
        input(),
        |decoded, response_headers, hidden_headers| {
            record.lock().unwrap().push(value(&[
                ("decoded", decoded),
                (
                    "response_headers",
                    serde_json::to_value(response_headers).unwrap(),
                ),
                (
                    "hidden_headers",
                    serde_json::to_value(hidden_headers).unwrap(),
                ),
            ]));
            Ok(text("projected"))
        },
    );
    let recorded = events.into_inner().unwrap_or_else(PoisonError::into_inner);
    let projected = recorded.first().cloned().unwrap_or(Value::Null);
    match result {
        Ok(returned) => Ok((returned, projected)),
        Err(error) => Err((error, projected)),
    }
}

const BODY: &[u8] =
    br#"{"id":"c","object":"chat.completion","created":7,"model":"m","choices":[]}"#;

pub(super) fn response_headers() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let env = environment(&redactor);
    let mut state = logging(&redactor);
    let (returned, projected) = respond(&mut state, &env, 200, BODY).unwrap();
    assert_eq!(returned, text("projected"));
    assert_eq!(projected["decoded"]["id"], text("c"));
    assert_eq!(projected["response_headers"], headers_value());
    assert_eq!(projected["hidden_headers"], projected["response_headers"]);
    recorded_post_call(&state);
    malformed_reply(&redactor, &env);
    class_keeps_status_and_headers();
}

fn recorded_post_call(state: &LoggingState) {
    let details = &state.model_call_details;
    let body = str::from_utf8(BODY).unwrap();
    assert_eq!(
        details.get("original_response"),
        Some(&Json::Text(body.to_owned()))
    );
    assert_eq!(details.get("input"), Some(&input().messages));
    let recorded = serde_json::to_value(details.get("additional_args").unwrap()).unwrap();
    assert_eq!(recorded["complete_input_dict"]["model"], text("gpt-4o"));
}

fn malformed_reply(redactor: &Redactor, env: &PrePostEnv<'_>) {
    let mut state = logging(redactor);
    let (error, _projected) = respond(&mut state, env, 400, b"not json").unwrap_err();
    match error {
        TransformResponseError::Json(record) => {
            assert_eq!(record.status_code, 400);
            assert_eq!(record.headers, Some(headers()));
        }
        TransformResponseError::BodyNotUtf8 => panic!("unexpected BodyNotUtf8"),
        TransformResponseError::Projection(_error) => panic!("unexpected projection failure"),
        TransformResponseError::Base { .. } => panic!("unexpected base failure"),
    }
    assert!(state.model_call_details.contains_key("original_response"));
}

fn class_keeps_status_and_headers() {
    let class = error_class(429, String::from("slow down"), Some(headers()));
    assert_eq!(class.status_code, 429);
    assert_eq!(class.headers, Some(headers()));
    assert_eq!(class.message, "slow down");
}
