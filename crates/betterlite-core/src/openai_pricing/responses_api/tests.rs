#![cfg(test)]

use std::io::Error as IoError;
use std::mem;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::response_events::{RawResponse, ReplyBody, ResponsesReply};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{ResponsesApiError, ResponsesCostRequest, responses_with_cost};
use crate::openai_pricing::{CompletionCost, CompletionCostRequest, CostError, CostedResponse};
use crate::routes::CallType;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: i64 = 9;
const REPLY: &str = r#"{"id":"r","created_at":5,"output":[]}"#;
const MODEL: &str = "gpt-4o";
const PROVIDER: &str = "openai";

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
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

#[derive(Debug, PartialEq)]
struct Observed {
    model: String,
    provider: String,
    call_type: CallType,
    embedding: bool,
}

struct RecordingCost {
    charge: Decimal,
    fails: bool,
    observed: Mutex<Vec<Observed>>,
}

impl RecordingCost {
    fn ok(charge: Decimal) -> Self {
        Self {
            charge,
            fails: false,
            observed: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            charge: Decimal::ZERO,
            fails: true,
            observed: Mutex::new(Vec::new()),
        }
    }

    fn observed(&self) -> Vec<Observed> {
        mem::take(&mut *self.observed.lock().unwrap())
    }
}

impl CompletionCost for RecordingCost {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Result<Decimal, CostError> {
        self.observed.lock().unwrap().push(Observed {
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.to_owned(),
            call_type: request.call_type,
            embedding: matches!(request.response, CostedResponse::Embedding(_)),
        });
        if self.fails {
            return Err(Box::new(IoError::other("no price for the model")));
        }
        Ok(self.charge)
    }
}

fn run(
    text: &str,
    cost: &RecordingCost,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<(ResponsesReply, Decimal), ResponsesApiError> {
    let headers = IndexMap::new();
    let reply = RawResponse {
        status: 200,
        headers: &headers,
        text,
    };
    responses_with_cost(
        ResponsesCostRequest {
            reply: &reply,
            model: MODEL,
            custom_llm_provider: PROVIDER,
            now: NOW,
            state,
            env,
        },
        cost,
    )
}

fn decimal(text: &str) -> Decimal {
    Decimal::from_str_exact(text).unwrap()
}

#[test]
fn a_typed_reply_is_costed_with_the_responses_call_type() {
    let redactor = redactor();
    let (mut state, env) = fixture(&redactor);
    let cost = RecordingCost::ok(decimal("0.5"));
    let (reply, charge) = run(REPLY, &cost, &mut state, &env).unwrap();
    assert!(matches!(reply.response, ReplyBody::Typed(_)));
    assert_eq!(charge, decimal("0.5"));
    assert_eq!(
        cost.observed(),
        vec![Observed {
            model: String::from(MODEL),
            provider: String::from(PROVIDER),
            call_type: CallType::Responses,
            embedding: false,
        }]
    );
}

#[test]
fn the_model_and_provider_are_passed_as_given() {
    let redactor = redactor();
    let (mut state, env) = fixture(&redactor);
    let cost = RecordingCost::ok(decimal("0.25"));
    let headers = IndexMap::new();
    let reply = RawResponse {
        status: 200,
        headers: &headers,
        text: REPLY,
    };
    responses_with_cost(
        ResponsesCostRequest {
            reply: &reply,
            model: "azure/gpt-4o",
            custom_llm_provider: "azure",
            now: NOW,
            state: &mut state,
            env: &env,
        },
        &cost,
    )
    .unwrap();
    assert_eq!(
        cost.observed(),
        vec![Observed {
            model: String::from("azure/gpt-4o"),
            provider: String::from("azure"),
            call_type: CallType::Responses,
            embedding: false,
        }]
    );
}

#[test]
fn a_reply_that_fails_validation_is_still_costed() {
    let redactor = redactor();
    let (mut state, env) = fixture(&redactor);
    let cost = RecordingCost::ok(decimal("0.5"));
    let (reply, charge) = run(
        r#"{"id":7,"created_at":5,"output":"not-a-list"}"#,
        &cost,
        &mut state,
        &env,
    )
    .unwrap();
    assert!(matches!(reply.response, ReplyBody::Lax(_)));
    assert_eq!(charge, decimal("0.5"));
}

#[rstest]
#[case::invalid_json("{not json")]
#[case::not_an_object("[]")]
#[case::missing_created_at(r#"{"id":"r"}"#)]
fn a_reply_that_cannot_be_transformed_is_not_costed(#[case] text: &str) {
    let redactor = redactor();
    let (mut state, env) = fixture(&redactor);
    let cost = RecordingCost::ok(decimal("0.5"));
    let Err(error) = run(text, &cost, &mut state, &env) else {
        panic!("the reply must fail to transform");
    };
    assert!(matches!(error, ResponsesApiError::Transform(_)));
    assert!(cost.observed().is_empty());
}

#[test]
fn a_failed_cost_is_returned_and_drops_the_reply() {
    let redactor = redactor();
    let (mut state, env) = fixture(&redactor);
    let cost = RecordingCost::failing();
    let Err(error) = run(REPLY, &cost, &mut state, &env) else {
        panic!("the cost must fail");
    };
    assert!(matches!(error, ResponsesApiError::Cost { .. }));
    assert_eq!(cost.observed().len(), 1);
}
