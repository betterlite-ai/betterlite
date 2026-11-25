#![cfg(test)]

use std::io;
use std::sync::Arc;
use std::thread::current;

use indexmap::IndexMap;

use super::super::context::PassThroughStreamContext;
use super::super::failure::StreamFailure;
use super::super::failure_scheduling::{FailureSchedulingCall, schedule_failure_logging};
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_stream_lifecycle::lifecycle::StreamLifecycle;
use crate::passthrough_stream_lifecycle::ports::StreamJobKind;
use crate::passthrough_stream_lifecycle::support::{
    Harness, RecordedFailure, capture_logs, json, model_details, object, recorded_usage,
    standard_outcome, time, usage_outcome,
};
use crate::passthrough_stream_logging::types::{EndpointType, HandlerError};

#[derive(Debug, thiserror::Error)]
#[error("the reconstruction failed")]
struct ReconstructionFailed;

fn failure() -> StreamFailure {
    StreamFailure::upstream(
        "ScriptedReadError".to_owned(),
        "the upstream read failed".to_owned(),
        "Traceback: scripted".to_owned(),
        Arc::new(io::Error::other("the upstream read failed")),
    )
}

fn stream_context(lifecycle: &StreamLifecycle) -> PassThroughStreamContext {
    PassThroughStreamContext {
        success_handler: Arc::clone(&lifecycle.passthrough),
        url_route: "/v1/messages".to_owned(),
        start_time: time("2024-01-01T00:00:00+00:00"),
    }
}

fn chunks() -> Vec<Vec<u8>> {
    vec![
        b"data: {\"usage\": {}}\n\n".to_vec(),
        b"data: [DONE]\n\n".to_vec(),
    ]
}

#[tokio::test]
async fn a_tinyfish_poller_owns_the_failure_row() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"]).with_tinyfish(true, None);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let context = stream_context(lifecycle.as_ref());
    let body = json("{}");
    let failure = failure();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::Tinyfish,
        request_body: &body,
        raw_bytes: &chunks(),
        failure: &failure,
        stream_context: Some(&context),
    })
    .await;
    assert!(harness.failure_dispatch.failures().is_empty());
    assert!(harness.queue.kinds().is_empty());
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
    assert_eq!(harness.handler.called(), 0);
}

#[tokio::test]
async fn a_tinyfish_stream_without_a_poller_still_dispatches_the_failure() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let context = stream_context(lifecycle.as_ref());
    let body = json("{}");
    let failure = failure();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::Tinyfish,
        request_body: &body,
        raw_bytes: &chunks(),
        failure: &failure,
        stream_context: Some(&context),
    })
    .await;
    assert_eq!(harness.failure_dispatch.failures().len(), 1);
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Failure]);
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
}

#[tokio::test]
async fn a_rejected_failure_job_is_logged_instead_of_raised() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"]).with_rejecting_queue();
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let body = json("{}");
    let failure = failure();
    let (logs, guard) = capture_logs();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::OpenAi,
        request_body: &body,
        raw_bytes: &chunks(),
        failure: &failure,
        stream_context: None,
    })
    .await;
    drop(guard);
    assert_eq!(harness.failure_dispatch.failures().len(), 1);
    let text = logs.text();
    assert!(
        text.contains("Error scheduling stream failure logging: the job was dropped (QueueFull)"),
        "{text}"
    );
}

#[tokio::test]
async fn a_recovered_partial_usage_is_recorded_with_the_response_cost() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"])
        .with_handler_reply(Ok(usage_outcome(2, 3, 5, "0.25")));
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let context = stream_context(lifecycle.as_ref());
    let body = json("{\"model\":\"gpt-4o\"}");
    let failure = failure();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::OpenAi,
        request_body: &body,
        raw_bytes: &chunks(),
        failure: &failure,
        stream_context: Some(&context),
    })
    .await;
    assert_eq!(recorded_usage(logging_obj.as_ref()), Some((2, 3, 5)));
    assert_eq!(
        model_details(logging_obj.as_ref()).get("response_cost"),
        Some(&json("0.25"))
    );
    assert_eq!(
        harness.failure_dispatch.failures(),
        vec![RecordedFailure {
            class: "ScriptedReadError".to_owned(),
            message: "the upstream read failed".to_owned(),
            traceback: "Traceback: scripted".to_owned(),
            retains_io_source: true,
        }]
    );
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Failure]);
    assert_eq!(harness.binder.calls(), 0);
}

#[tokio::test]
async fn the_recovered_usage_handler_runs_off_the_reactor_thread() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"])
        .with_handler_reply(Ok(usage_outcome(2, 3, 5, "0.25")));
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let context = stream_context(lifecycle.as_ref());
    let body = json("{\"model\":\"gpt-4o\"}");
    let failure = failure();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::OpenAi,
        request_body: &body,
        raw_bytes: &chunks(),
        failure: &failure,
        stream_context: Some(&context),
    })
    .await;
    let calls = harness.handler.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url_route, "/v1/messages");
    assert_eq!(calls[0].request_body, Some(body));
    assert_eq!(calls[0].endpoint_type, EndpointType::OpenAi);
    assert_eq!(
        calls[0].lines,
        vec![
            "data: {\"usage\": {}}".to_owned(),
            "data: [DONE]".to_owned()
        ]
    );
    assert_eq!(calls[0].model, None);
    assert_eq!(calls[0].start_time, time("2024-01-01T00:00:00+00:00"));
    assert_eq!(calls[0].end_time, time("2024-01-01T00:00:02+00:00"));
    assert_ne!(calls[0].thread, current().id());
}

#[tokio::test]
async fn an_anthropic_stream_goes_to_the_dedicated_handler() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let body = json("{\"model\":\"claude-3\"}");
    let failure = failure();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::Anthropic,
        request_body: &body,
        raw_bytes: &chunks(),
        failure: &failure,
        stream_context: None,
    })
    .await;
    assert_eq!(
        *harness.anthropic.calls.lock().unwrap(),
        vec![(body, chunks())]
    );
    assert_eq!(harness.handler.called(), 0);
    assert_eq!(harness.failure_dispatch.failures().len(), 1);
}

#[tokio::test]
async fn without_a_stream_context_nothing_is_reconstructed() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"]);
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let body = json("{}");
    let failure = failure();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::OpenAi,
        request_body: &body,
        raw_bytes: &chunks(),
        failure: &failure,
        stream_context: None,
    })
    .await;
    assert_eq!(harness.handler.called(), 0);
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
    assert_eq!(harness.failure_dispatch.failures().len(), 1);
}

#[tokio::test]
async fn empty_raw_bytes_are_not_reconstructed() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"])
        .with_handler_reply(Ok(usage_outcome(2, 3, 5, "0.25")));
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let context = stream_context(lifecycle.as_ref());
    let body = json("{}");
    let failure = failure();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::OpenAi,
        request_body: &body,
        raw_bytes: &[],
        failure: &failure,
        stream_context: Some(&context),
    })
    .await;
    assert_eq!(harness.handler.called(), 0);
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
    assert_eq!(harness.failure_dispatch.failures().len(), 1);
}

#[tokio::test]
async fn a_reconstruction_error_is_skipped_without_failing_the_dispatch() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"])
        .with_handler_reply(Err(HandlerError::leaf(ReconstructionFailed)));
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let context = stream_context(lifecycle.as_ref());
    let body = json("{}");
    let failure = failure();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::OpenAi,
        request_body: &body,
        raw_bytes: &chunks(),
        failure: &failure,
        stream_context: Some(&context),
    })
    .await;
    assert_eq!(harness.handler.called(), 1);
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
    assert_eq!(harness.failure_dispatch.failures().len(), 1);
    assert_eq!(harness.queue.kinds(), vec![StreamJobKind::Failure]);
}

#[tokio::test]
async fn a_reconstructed_response_without_usage_records_nothing() {
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"])
        .with_handler_reply(Ok(standard_outcome("no usage")));
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let context = stream_context(lifecycle.as_ref());
    let body = json("{}");
    let failure = failure();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::OpenAi,
        request_body: &body,
        raw_bytes: &chunks(),
        failure: &failure,
        stream_context: Some(&context),
    })
    .await;
    assert!(recorded_usage(logging_obj.as_ref()).is_none());
    assert!(
        model_details(logging_obj.as_ref())
            .get("response_cost")
            .is_none()
    );
    assert_eq!(harness.failure_dispatch.failures().len(), 1);
}

#[tokio::test]
async fn a_missing_response_cost_records_zero() {
    let mut outcome = usage_outcome(1, 2, 3, "0.0");
    outcome.kwargs = PassThroughLoggingKwargs::default();
    let harness = Harness::new(&["2024-01-01T00:00:02+00:00"]).with_handler_reply(Ok(outcome));
    let lifecycle = harness.lifecycle(false);
    let logging_obj = object(&[], IndexMap::new());
    let context = stream_context(lifecycle.as_ref());
    let body = json("{}");
    let failure = failure();
    schedule_failure_logging(FailureSchedulingCall {
        lifecycle: lifecycle.as_ref(),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::OpenAi,
        request_body: &body,
        raw_bytes: &chunks(),
        failure: &failure,
        stream_context: Some(&context),
    })
    .await;
    assert_eq!(recorded_usage(logging_obj.as_ref()), Some((1, 2, 3)));
    assert_eq!(
        model_details(logging_obj.as_ref()).get("response_cost"),
        Some(&json("0.0"))
    );
}
