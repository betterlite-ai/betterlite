#![cfg(test)]

use std::io;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::logging_success::ports::SuccessClock;
use crate::passthrough_stream_lifecycle::context::PassThroughStreamContext;
use crate::passthrough_stream_lifecycle::partial_usage::AnthropicPartialUsage;
use crate::passthrough_stream_lifecycle::support::{
    Harness, RecordingAnthropic, StepClock, capture_logs, json, object, passthrough,
    recorded_usage, response_cost, standard_outcome, time, usage_outcome,
    usage_without_cost_outcome,
};
use crate::passthrough_stream_logging::object::StreamLoggingObject;
use crate::passthrough_stream_logging::types::{EndpointType, HandlerError};

use super::{PartialUsageCall, endpoint_value, record_partial_usage_failure};

const ROUTE: &str = "/v1/chat/completions";

fn context() -> PassThroughStreamContext {
    PassThroughStreamContext {
        success_handler: passthrough(),
        url_route: ROUTE.to_owned(),
        start_time: time("2024-01-01T00:00:00+00:00"),
    }
}

#[test]
fn response_cost_conversion_rejects_values_outside_decimal_range() {
    let number = "1e999".parse::<serde_json::Number>().unwrap();
    let mut kwargs =
        crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs::default();
    kwargs
        .entries
        .insert("response_cost".to_owned(), Json::Number(number));
    assert!(super::super::response_cost(&kwargs).is_err());
}

struct Case {
    endpoint_type: EndpointType,
    request_body: Json,
    raw_bytes: Vec<Vec<u8>>,
    stream_context: Option<PassThroughStreamContext>,
}

fn call(harness: &Harness, logging_obj: Arc<StreamLoggingObject>, case: Case) -> PartialUsageCall {
    let anthropic: Arc<dyn AnthropicPartialUsage> =
        Arc::<RecordingAnthropic>::clone(&harness.anthropic);
    let clock: Arc<dyn SuccessClock> = Arc::<StepClock>::clone(&harness.clock);
    PartialUsageCall {
        logging_obj,
        passthrough: passthrough(),
        providers: Arc::clone(&harness.providers),
        anthropic,
        clock,
        endpoint_type: case.endpoint_type,
        request_body: case.request_body,
        raw_bytes: case.raw_bytes,
        stream_context: case.stream_context,
    }
}

fn body() -> Json {
    json(r#"{"model":"gpt-4o"}"#)
}

#[test]
fn an_anthropic_stream_delegates_to_the_anthropic_handler() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"]);
    let logging_obj = object(&[], IndexMap::new());
    let chunks = vec![b"event: message_start\n\n".to_vec()];
    record_partial_usage_failure(&call(
        &harness,
        Arc::clone(&logging_obj),
        Case {
            endpoint_type: EndpointType::Anthropic,
            request_body: body(),
            raw_bytes: chunks.clone(),
            stream_context: Some(context()),
        },
    ));
    assert_eq!(
        *harness.anthropic.calls.lock().unwrap(),
        vec![(body(), chunks)]
    );
    assert_eq!(harness.handler.called(), 0);
    assert_eq!(harness.clock.calls(), 0);
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
}

#[test]
fn an_anthropic_stream_delegates_before_the_empty_bytes_check() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"]);
    let logging_obj = object(&[], IndexMap::new());
    record_partial_usage_failure(&call(
        &harness,
        Arc::clone(&logging_obj),
        Case {
            endpoint_type: EndpointType::Anthropic,
            request_body: body(),
            raw_bytes: Vec::new(),
            stream_context: None,
        },
    ));
    assert_eq!(
        *harness.anthropic.calls.lock().unwrap(),
        vec![(body(), Vec::new())]
    );
    assert_eq!(harness.handler.called(), 0);
    assert_eq!(harness.clock.calls(), 0);
}

#[test]
fn no_stream_context_records_nothing() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"])
        .with_handler_reply(Ok(usage_outcome(10, 5, 15, "0.25")));
    let logging_obj = object(&[], IndexMap::new());
    record_partial_usage_failure(&call(
        &harness,
        Arc::clone(&logging_obj),
        Case {
            endpoint_type: EndpointType::OpenAi,
            request_body: body(),
            raw_bytes: vec![b"data: one\n\n".to_vec()],
            stream_context: None,
        },
    ));
    assert_eq!(harness.handler.called(), 0);
    assert_eq!(harness.clock.calls(), 0);
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
    assert!(response_cost(logging_obj.as_ref()).is_none());
}

#[test]
fn empty_raw_bytes_record_nothing() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"])
        .with_handler_reply(Ok(usage_outcome(10, 5, 15, "0.25")));
    let logging_obj = object(&[], IndexMap::new());
    record_partial_usage_failure(&call(
        &harness,
        Arc::clone(&logging_obj),
        Case {
            endpoint_type: EndpointType::OpenAi,
            request_body: body(),
            raw_bytes: Vec::new(),
            stream_context: Some(context()),
        },
    ));
    assert_eq!(harness.handler.called(), 0);
    assert_eq!(harness.clock.calls(), 0);
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
}

#[test]
fn the_recovered_usage_and_cost_are_recorded() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"])
        .with_handler_reply(Ok(usage_outcome(10, 5, 15, "0.25")));
    let logging_obj = object(&[], IndexMap::new());
    record_partial_usage_failure(&call(
        &harness,
        Arc::clone(&logging_obj),
        Case {
            endpoint_type: EndpointType::OpenAi,
            request_body: body(),
            raw_bytes: vec![b"data: one\n\n".to_vec()],
            stream_context: Some(context()),
        },
    ));
    assert_eq!(harness.clock.calls(), 1);
    assert_eq!(recorded_usage(logging_obj.as_ref()), Some((10, 5, 15)));
    assert_eq!(response_cost(logging_obj.as_ref()), Some(json("0.25")));
    let calls = harness.handler.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url_route, ROUTE);
    assert_eq!(calls[0].request_body, Some(body()));
    assert_eq!(calls[0].endpoint_type, EndpointType::OpenAi);
    assert_eq!(calls[0].start_time, time("2024-01-01T00:00:00+00:00"));
    assert_eq!(calls[0].end_time, time("2024-01-01T00:00:02+00:00"));
    assert_eq!(calls[0].lines, vec!["data: one".to_owned()]);
}

#[test]
fn a_response_without_usage_records_nothing() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"])
        .with_handler_reply(Ok(standard_outcome("done")));
    let logging_obj = object(&[], IndexMap::new());
    record_partial_usage_failure(&call(
        &harness,
        Arc::clone(&logging_obj),
        Case {
            endpoint_type: EndpointType::OpenAi,
            request_body: body(),
            raw_bytes: vec![b"data: one\n\n".to_vec()],
            stream_context: Some(context()),
        },
    ));
    assert_eq!(harness.handler.called(), 1);
    assert_eq!(harness.clock.calls(), 1);
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
    assert!(response_cost(logging_obj.as_ref()).is_none());
}

#[test]
fn a_response_without_a_cost_records_zero() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"])
        .with_handler_reply(Ok(usage_without_cost_outcome(3, 4, 7)));
    let logging_obj = object(&[], IndexMap::new());
    record_partial_usage_failure(&call(
        &harness,
        Arc::clone(&logging_obj),
        Case {
            endpoint_type: EndpointType::OpenAi,
            request_body: body(),
            raw_bytes: vec![b"data: one\n\n".to_vec()],
            stream_context: Some(context()),
        },
    ));
    assert_eq!(recorded_usage(logging_obj.as_ref()), Some((3, 4, 7)));
    assert_eq!(response_cost(logging_obj.as_ref()), Some(json("0.0")));
}

#[test]
fn a_reconstruction_error_is_logged_with_the_endpoint_label() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"]).with_handler_reply(Err(
        HandlerError::leaf(io::Error::other("handler exploded")),
    ));
    let logging_obj = object(&[], IndexMap::new());
    let (logs, guard) = capture_logs();
    record_partial_usage_failure(&call(
        &harness,
        Arc::clone(&logging_obj),
        Case {
            endpoint_type: EndpointType::OpenAi,
            request_body: body(),
            raw_bytes: vec![b"data: one\n\n".to_vec()],
            stream_context: Some(context()),
        },
    ));
    drop(guard);
    let text = logs.text();
    assert!(
        text.contains(
            "Could not recover the partial usage of a failed openai pass-through stream: handler exploded"
        ),
        "{text}"
    );
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
}

#[test]
fn the_endpoint_labels_match_the_litellm_values() {
    assert_eq!(endpoint_value(EndpointType::VertexAi), "vertex-ai");
    assert_eq!(endpoint_value(EndpointType::Gemini), "gemini");
    assert_eq!(endpoint_value(EndpointType::Anthropic), "anthropic");
    assert_eq!(endpoint_value(EndpointType::OpenAi), "openai");
    assert_eq!(endpoint_value(EndpointType::Tinyfish), "tinyfish");
    assert_eq!(endpoint_value(EndpointType::Generic), "generic");
    assert_eq!(endpoint_value(EndpointType::Decisions), "decisions");
}
