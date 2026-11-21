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
use betterlite_models::decisions::Json as HiddenJson;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::Value;
use uuid::Uuid;

use crate::chat_choices::reply::{ResponseReply, TransformResponseError};
use crate::compatible_wire::{LogInput, ReplyOptions};
use crate::edenai_common::RESPONSE_COST_HEADER;
use crate::openai_base_config::Headers;
use crate::openai_base_config::forwarding::ForwardingFailure;

use super::super::nadir::{self, NadirReply, ReplyRequest};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

// the marker lets a test inject a projection failure after the reply decodes
fn projection(
    decoded: Value,
    _response: &Headers,
    _hidden: &Headers,
) -> Result<Value, ForwardingFailure> {
    if decoded.get("refuse_projection").is_some() {
        return Err(ForwardingFailure {
            message: String::from("the test projection refused the reply"),
        });
    }
    Ok(decoded)
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn fixture(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
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
        now: DateTime::parse_from_rfc3339("2026-01-01T00:05:00+00:00").unwrap(),
    };
    (state, env)
}

fn log_input() -> LogInput {
    LogInput {
        messages: HiddenJson::List(vec![HiddenJson::Object(IndexMap::from([(
            String::from("role"),
            HiddenJson::Text(String::from("user")),
        )]))]),
        data: RawBody::Object(IndexMap::from([(
            String::from("model"),
            HiddenJson::Text(String::from("m")),
        )])),
    }
}

fn reply_options() -> ReplyOptions<'static> {
    ReplyOptions {
        json_mode: false,
        provider: Some("nadir"),
        base_model: None,
        identifier: Uuid::nil,
        clock: || 7,
    }
}

fn make_redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

fn nadir_reply(body: &[u8]) -> Result<NadirReply, TransformResponseError> {
    let redactor = make_redactor();
    let (mut state, env) = fixture(&redactor);
    let headers = Headers::from([(String::from("x-request-id"), String::from("req-1"))]);
    nadir::transform_response(
        ReplyRequest {
            reply: ResponseReply {
                status: 200,
                headers: &headers,
                body,
            },
            state: &mut state,
            env: &env,
            input: log_input(),
            options: reply_options(),
        },
        projection,
    )
}

#[test]
fn nadir_records_the_cost_after_the_parent_reply() {
    let reply = nadir_reply(br#"{"choices":[],"nadir_metadata":{"cost":{"total_cost_usd":0.25}}}"#)
        .expect("the parent transforms the reply");
    assert_eq!(
        reply.response.get("choices"),
        Some(&Value::Array(Vec::new()))
    );
    let hidden = serde_json::to_value(&reply.hidden_params).unwrap();
    assert_eq!(
        hidden["additional_headers"][RESPONSE_COST_HEADER],
        Value::from(0.25_f64)
    );
}

#[test]
fn nadir_records_no_cost_when_the_bytes_carry_none() {
    let reply = nadir_reply(br#"{"choices":[],"nadir_metadata":{"cost":{"total_cost_usd":true}}}"#)
        .expect("the parent transforms the reply");
    assert!(reply.hidden_params.is_empty());
}

#[test]
fn nadir_returns_the_parent_error_before_reading_any_cost() {
    let result = nadir_reply(b"not json");
    assert!(matches!(result, Err(TransformResponseError::Json(_))));
}

#[test]
fn nadir_returns_a_projection_failure_before_reading_any_cost() {
    let result = nadir_reply(
        br#"{"refuse_projection":true,"nadir_metadata":{"cost":{"total_cost_usd":0.25}}}"#,
    );
    assert!(matches!(result, Err(TransformResponseError::Projection(_))));
}
