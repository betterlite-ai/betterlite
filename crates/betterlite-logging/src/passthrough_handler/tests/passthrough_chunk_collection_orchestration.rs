use betterlite_models::decisions::Json;
use rstest::rstest;
use tracing::Level;

use super::support::{capture_logs, cost, cost_json, end, harness, json, json_object, start};
use crate::passthrough_handler::types::{
    CollectedChunksInput, NO_COMPLETE_RESPONSE, PAYLOAD_ERROR, StreamedResponse,
};
use crate::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use crate::passthrough_logging_state::response::PassThroughResponse;

#[test]
fn builds_the_payload_for_the_rebuilt_response() {
    let harness = harness();
    harness
        .provider
        .stream(Ok(Some(StreamedResponse::Model(json(
            r#"{"id": "rebuilt-9"}"#,
        )))));
    harness.cost.costs(cost("0.5"));
    let request = json_object(r#"{"model": "claude-3"}"#);
    let chunks = vec!["one".to_owned(), "two".to_owned()];
    let outcome = harness
        .handler
        .handle_collected_chunks(CollectedChunksInput {
            logging_obj: &harness.logging,
            request_body: &request,
            all_chunks: &chunks,
            start_time: start(),
            end_time: end(),
        })
        .unwrap();
    let calls = harness.provider.chunks();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "claude-3");
    assert_eq!(calls[0].chunks, chunks);
    assert_eq!(calls[0].call_id.as_deref(), Some("call-1"));
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(json(
            r#"{"id": "call-1", "model": "claude-3"}"#
        )))
    );
    assert_eq!(
        outcome.kwargs.entries["response_cost"],
        cost_json(cost("0.5"))
    );
    assert_eq!(
        outcome.kwargs.entries["model"],
        Json::Text("claude-3".to_owned())
    );
    assert!(
        !outcome
            .kwargs
            .entries
            .contains_key("passthrough_logging_payload")
    );
    assert_eq!(
        harness.cost.calls()[0].response,
        json(r#"{"id": "rebuilt-9"}"#)
    );
}

#[test]
fn returns_no_result_when_the_rebuild_yields_nothing() {
    let harness = harness();
    harness.provider.stream(Ok(None));
    let (captured, _guard) = capture_logs(Level::ERROR);
    let request = json_object(r#"{"model": "claude-3"}"#);
    let outcome = harness
        .handler
        .handle_collected_chunks(CollectedChunksInput {
            logging_obj: &harness.logging,
            request_body: &request,
            all_chunks: &["one".to_owned()],
            start_time: start(),
            end_time: end(),
        })
        .unwrap();
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, PassThroughLoggingKwargs::default());
    assert!(harness.cost.calls().is_empty());
    assert!(captured.text().contains(NO_COMPLETE_RESPONSE));
}

#[rstest]
#[case::absent(r#"{}"#)]
#[case::null(r#"{"model": null}"#)]
fn uses_an_empty_model_when_the_body_has_none(#[case] body: &str) {
    let harness = harness();
    harness
        .provider
        .stream(Ok(Some(StreamedResponse::Model(json(
            r#"{"id": "rebuilt-9"}"#,
        )))));
    harness.cost.costs(cost("0.5"));
    let request = json_object(body);
    let outcome = harness
        .handler
        .handle_collected_chunks(CollectedChunksInput {
            logging_obj: &harness.logging,
            request_body: &request,
            all_chunks: &["one".to_owned()],
            start_time: start(),
            end_time: end(),
        })
        .unwrap();
    assert_eq!(harness.provider.chunks()[0].model, "");
    assert_eq!(outcome.kwargs.entries["model"], Json::Text(String::new()));
}

#[test]
fn propagates_a_rebuild_error() {
    let harness = harness();
    harness.provider.stream_fails();
    let request = json_object(r#"{"model": "claude-3"}"#);
    let error = harness
        .handler
        .handle_collected_chunks(CollectedChunksInput {
            logging_obj: &harness.logging,
            request_body: &request,
            all_chunks: &[],
            start_time: start(),
            end_time: end(),
        })
        .expect_err("the rebuild failure is not caught");
    assert_eq!(error.to_string(), "the fake probe failed");
    assert!(harness.cost.calls().is_empty());
}

#[test]
fn refuses_a_non_text_model_before_the_rebuild() {
    let harness = harness();
    let request = json_object(r#"{"model": ["claude-3"]}"#);
    let error = harness
        .handler
        .handle_collected_chunks(CollectedChunksInput {
            logging_obj: &harness.logging,
            request_body: &request,
            all_chunks: &[],
            start_time: start(),
            end_time: end(),
        })
        .expect_err("a list is not a model");
    assert_eq!(
        error.to_string(),
        "the request_body.model value must be a string"
    );
    assert!(harness.provider.chunks().is_empty());
}

#[test]
fn rebuilds_a_text_completion() {
    let harness = harness();
    harness
        .provider
        .stream(Ok(Some(StreamedResponse::TextCompletion(json(
            r#"{"id": "rebuilt-9"}"#,
        )))));
    harness.cost.costs(cost("0.5"));
    let request = json_object(r#"{"model": "claude-3"}"#);
    let outcome = harness
        .handler
        .handle_collected_chunks(CollectedChunksInput {
            logging_obj: &harness.logging,
            request_body: &request,
            all_chunks: &[],
            start_time: start(),
            end_time: end(),
        })
        .unwrap();
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::TextCompletion(json(
            r#"{"id": "call-1", "model": "claude-3"}"#
        )))
    );
}

#[test]
fn a_payload_failure_keeps_the_rebuilt_response() {
    let harness = harness();
    harness
        .provider
        .stream(Ok(Some(StreamedResponse::Model(json(
            r#"{"id": "rebuilt-9"}"#,
        )))));
    let (captured, _guard) = capture_logs(Level::ERROR);
    let request = json_object(r#"{"model": "claude-3"}"#);
    let outcome = harness
        .handler
        .handle_collected_chunks(CollectedChunksInput {
            logging_obj: &harness.logging,
            request_body: &request,
            all_chunks: &[],
            start_time: start(),
            end_time: end(),
        })
        .unwrap();
    assert_eq!(
        outcome.result,
        Some(PassThroughResponse::ModelResponse(json(
            r#"{"id": "rebuilt-9"}"#
        )))
    );
    assert!(outcome.kwargs.entries.is_empty());
    assert!(captured.text().contains(PAYLOAD_ERROR));
}
