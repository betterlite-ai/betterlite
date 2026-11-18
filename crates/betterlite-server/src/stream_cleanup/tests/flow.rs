use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::super::ports::CleanupError;
use super::support::{
    Ending, FakeLogging, FakeSlots, backfilled_usage, billable_outcome, capture_logs,
    cleanup_outcome, disconnect_cleanup, number, object, snapshot, text,
};
use crate::sse_data_generator::types::StreamCleanup;

#[tokio::test]
async fn run_cleanup_bills_a_partial_response_with_backfilled_usage() {
    let partial = object(&[
        ("model", text("alias")),
        (
            "usage",
            Json::Object(object(&[
                ("prompt_tokens", number(10)),
                ("completion_tokens", number(5)),
                ("total_tokens", number(15)),
                (
                    "prompt_tokens_details",
                    Json::Object(object(&[("cached_tokens", number(4))])),
                ),
            ])),
        ),
    ]);
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))).with_assemble(Ok(Some(partial))));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.run_cleanup(billable_outcome()).await.unwrap();

    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "assemble",
            "dispatch",
            "details",
            "baseline"
        ]
    );
    assert!(slots.keys().is_empty());
    assert_eq!(logging.assembled(), Some((1, Some(vec![text("hi")]), true)));
    assert_eq!(
        logging.dispatched(),
        vec![object(&[
            ("model", text("gpt-4o")),
            ("usage", backfilled_usage()),
        ])]
    );
}

#[tokio::test]
async fn run_cleanup_dispatches_a_partial_whose_usage_does_not_parse() {
    let usage = Json::Object(object(&[("prompt_tokens", text("ten"))]));
    let partial = object(&[("model", text("alias")), ("usage", usage.clone())]);
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))).with_assemble(Ok(Some(partial))));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.run_cleanup(billable_outcome()).await.unwrap();

    assert_eq!(
        logging.dispatched(),
        vec![object(&[("model", text("gpt-4o")), ("usage", usage)])]
    );
    assert!(slots.keys().is_empty());
}

#[tokio::test]
async fn run_cleanup_logs_the_disconnect_and_the_partial_billing() {
    let (buffer, guard) = capture_logs();
    let partial = object(&[("model", text("alias"))]);
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))).with_assemble(Ok(Some(partial))));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.run_cleanup(billable_outcome()).await.unwrap();
    drop(guard);

    let text = buffer.text();
    assert!(
        text.contains(
            "Recorded streaming client disconnect with error_code=499 for litellm_call_id=call-1"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "Billing partial streamed spend for 1 chunks after client disconnect, litellm_call_id=call-1"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn run_cleanup_releases_the_slot_when_streaming_logging_is_disabled() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let mut cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.disable_streaming_logging = true;
    cleanup.run_cleanup(billable_outcome()).await.unwrap();

    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "release:deployment-1",
            "details",
            "baseline"
        ]
    );
    assert_eq!(slots.keys(), ["deployment-1"]);
    assert_eq!(logging.assembled(), None);
    assert!(logging.dispatched().is_empty());
}

#[tokio::test]
async fn run_cleanup_releases_the_slot_without_a_logging_object() {
    let logging = Arc::new(FakeLogging::new(None));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.run_cleanup(billable_outcome()).await.unwrap();

    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "release:deployment-1",
            "details"
        ]
    );
    assert_eq!(slots.keys(), ["deployment-1"]);
    assert!(logging.dispatched().is_empty());
}

#[tokio::test]
async fn run_cleanup_releases_the_slot_without_a_response_handle() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup
        .run_cleanup(cleanup_outcome(Ending::Disconnected))
        .await
        .unwrap();

    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "release:deployment-1",
            "details",
            "baseline"
        ]
    );
    assert_eq!(slots.keys(), ["deployment-1"]);
}

#[tokio::test]
async fn run_cleanup_releases_the_slot_without_collected_chunks() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    let mut outcome = cleanup_outcome(Ending::Disconnected);
    outcome.response = Some(snapshot(Vec::new(), None, None));
    cleanup.run_cleanup(outcome).await.unwrap();

    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "release:deployment-1",
            "details",
            "baseline"
        ]
    );
    assert_eq!(slots.keys(), ["deployment-1"]);
    assert_eq!(logging.assembled(), None);
}

#[tokio::test]
async fn run_cleanup_releases_the_slot_when_assembly_returns_none() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.run_cleanup(billable_outcome()).await.unwrap();

    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "assemble",
            "release:deployment-1",
            "details",
            "baseline"
        ]
    );
    assert_eq!(slots.keys(), ["deployment-1"]);
    assert!(logging.dispatched().is_empty());
}

#[tokio::test]
async fn run_cleanup_releases_the_slot_when_assembly_fails() {
    let (buffer, guard) = capture_logs();
    let logging = Arc::new(
        FakeLogging::new(Some(object(&[])))
            .with_assemble(Err(CleanupError::Port("assembly failed".to_owned()))),
    );
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.run_cleanup(billable_outcome()).await.unwrap();
    drop(guard);

    let text = buffer.text();
    assert!(
        text.contains(
            "Failed to assemble partial streamed response for disconnect billing: assembly failed"
        ),
        "{text}"
    );
    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "assemble",
            "release:deployment-1",
            "details",
            "baseline"
        ]
    );
    assert_eq!(slots.keys(), ["deployment-1"]);
}

#[tokio::test]
async fn run_cleanup_releases_the_slot_when_dispatch_fails() {
    let (buffer, guard) = capture_logs();
    let logging = Arc::new(
        FakeLogging::new(Some(object(&[])))
            .with_assemble(Ok(Some(object(&[("model", text("alias"))]))))
            .with_dispatch(Err(CleanupError::Port("dispatch failed".to_owned()))),
    );
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.run_cleanup(billable_outcome()).await.unwrap();
    drop(guard);

    let text = buffer.text();
    assert!(
        text.contains("Failed to dispatch disconnect billing event: dispatch failed"),
        "{text}"
    );
    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "assemble",
            "dispatch",
            "release:deployment-1",
            "details",
            "baseline"
        ]
    );
    assert_eq!(slots.keys(), ["deployment-1"]);
}

#[tokio::test]
async fn run_cleanup_skips_billing_after_a_dispatched_final_success() {
    let details = object(&[("has_dispatched_final_stream_success", Json::Bool(true))]);
    let logging = Arc::new(FakeLogging::new(Some(details)));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.run_cleanup(billable_outcome()).await.unwrap();

    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "details",
            "baseline"
        ]
    );
    assert!(slots.keys().is_empty());
    assert_eq!(logging.assembled(), None);
}

#[rstest]
#[case::false_flag(Json::Bool(false))]
#[case::text_flag(Json::Text("true".to_owned()))]
#[case::null_flag(Json::Null)]
#[tokio::test]
async fn run_cleanup_bills_unless_the_final_success_flag_is_true(#[case] flag: Json) {
    let details = object(&[("has_dispatched_final_stream_success", flag)]);
    let logging = Arc::new(
        FakeLogging::new(Some(details))
            .with_assemble(Ok(Some(object(&[("model", text("alias"))])))),
    );
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.run_cleanup(billable_outcome()).await.unwrap();

    assert!(logging.events().contains(&"assemble".to_owned()));
    assert!(slots.keys().is_empty());
    assert_eq!(logging.dispatched().len(), 1);
}

#[tokio::test]
async fn run_cleanup_fires_deferred_logging_without_billing_or_release() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))).with_armed(true));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.run_cleanup(billable_outcome()).await.unwrap();

    assert_eq!(
        logging.events(),
        ["edit", "armed:true", "fire:true", "details", "baseline"]
    );
    assert_eq!(logging.fired(), 1);
    assert_eq!(logging.assembled(), None);
    assert!(logging.dispatched().is_empty());
    assert!(slots.keys().is_empty());
}

#[tokio::test]
async fn run_cleanup_invalidates_the_baseline_of_an_unfinished_stream() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))).with_baseline(true));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup
        .run_cleanup(cleanup_outcome(Ending::Disconnected))
        .await
        .unwrap();

    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "release:deployment-1",
            "details",
            "baseline",
            "invalidate:incomplete_response:true"
        ]
    );
}

#[tokio::test]
async fn run_cleanup_invalidates_the_baseline_after_a_failure() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))).with_baseline(true));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup
        .run_cleanup(cleanup_outcome(Ending::Failed))
        .await
        .unwrap();

    assert_eq!(
        logging.events(),
        ["details", "baseline", "invalidate:incomplete_response:true"]
    );
    assert!(slots.keys().is_empty());
    assert_eq!(logging.fired(), 0);
}

#[tokio::test]
async fn run_cleanup_keeps_the_baseline_of_a_completed_stream() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))).with_baseline(true));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup
        .run_cleanup(cleanup_outcome(Ending::Completed))
        .await
        .unwrap();

    assert!(logging.events().is_empty());
    assert!(slots.keys().is_empty());
}

#[rstest]
#[case::complete_flag(Json::Bool(true), vec!["details", "baseline"])]
#[case::text_flag(
    Json::Text("true".to_owned()),
    vec!["details", "baseline", "invalidate:incomplete_response:true"]
)]
#[tokio::test]
async fn run_cleanup_invalidates_unless_prompt_cache_complete_is_true(
    #[case] flag: Json,
    #[case] expected: Vec<&str>,
) {
    let details = object(&[("prompt_cache_response_complete", flag)]);
    let logging = Arc::new(FakeLogging::new(Some(details)).with_baseline(true));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup
        .run_cleanup(cleanup_outcome(Ending::Failed))
        .await
        .unwrap();

    assert_eq!(logging.events(), expected);
}

#[tokio::test]
async fn run_cleanup_keeps_the_baseline_when_it_is_inactive() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup
        .run_cleanup(cleanup_outcome(Ending::Failed))
        .await
        .unwrap();

    assert_eq!(logging.events(), ["details", "baseline"]);
}

#[tokio::test]
async fn run_cleanup_skips_the_baseline_without_a_logging_object() {
    let logging = Arc::new(FakeLogging::new(None));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup
        .run_cleanup(cleanup_outcome(Ending::Failed))
        .await
        .unwrap();

    assert_eq!(logging.events(), ["details"]);
}

#[tokio::test]
async fn run_cleanup_returns_before_the_baseline_when_the_release_fails() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))).with_baseline(true));
    let slots = Arc::new(FakeSlots::failing(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    let error = cleanup
        .run_cleanup(cleanup_outcome(Ending::Disconnected))
        .await
        .unwrap_err();

    assert_eq!(error, CleanupError::Port("release failed".to_owned()));
    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "release:deployment-1"
        ]
    );
}

#[tokio::test]
async fn run_cleanup_returns_an_invalidate_error() {
    let logging = Arc::new(
        FakeLogging::new(Some(object(&[])))
            .with_baseline(true)
            .with_invalidate(Err(CleanupError::Port("invalidate failed".to_owned()))),
    );
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    let error = cleanup
        .run_cleanup(cleanup_outcome(Ending::Failed))
        .await
        .unwrap_err();

    assert_eq!(error, CleanupError::Port("invalidate failed".to_owned()));
    assert_eq!(
        logging.events(),
        ["details", "baseline", "invalidate:incomplete_response:true"]
    );
}

#[tokio::test]
async fn finalize_runs_the_cleanup_once_on_its_tracker() {
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))));
    let slots = Arc::new(FakeSlots::attached_to(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.finalize(cleanup_outcome(Ending::Disconnected));
    cleanup.tracker.close();
    cleanup.tracker.wait().await;

    assert_eq!(logging.fired(), 1);
    assert_eq!(slots.keys(), ["deployment-1"]);
    assert_eq!(
        logging.events(),
        [
            "edit",
            "armed:false",
            "fire:true",
            "details",
            "release:deployment-1",
            "details",
            "baseline"
        ]
    );
}

#[tokio::test]
async fn finalize_logs_a_cleanup_error() {
    let (buffer, guard) = capture_logs();
    let logging = Arc::new(FakeLogging::new(Some(object(&[]))));
    let slots = Arc::new(FakeSlots::failing(&logging));
    let cleanup = disconnect_cleanup(Arc::clone(&logging), Arc::clone(&slots));
    cleanup.finalize(cleanup_outcome(Ending::Disconnected));
    cleanup.tracker.close();
    cleanup.tracker.wait().await;
    drop(guard);

    assert!(
        buffer
            .text()
            .contains("stream cleanup failed: release failed"),
        "{}",
        buffer.text()
    );
}
