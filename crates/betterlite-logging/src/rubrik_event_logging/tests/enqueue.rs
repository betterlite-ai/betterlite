use betterlite_logging::rubrik_delivery::queue::queue_snapshot;
use betterlite_logging::rubrik_event_logging::enqueue::enqueue_log_event;
use betterlite_logging::rubrik_event_logging::event::EventKind;
use betterlite_logging::rubrik_event_logging::success::async_log_success_event;
use betterlite_models::decisions::Json;
use rstest::rstest;
use tracing::Level;

use super::support::{Harness, ScriptedRandom, UnusedRandom, details, field, text};

const EVENT: &str = r#"{"litellm_call_id":"call-1","standard_logging_object":{"id":"chatcmpl-provider","status":"success"}}"#;

#[tokio::test]
async fn a_kept_success_event_is_queued() {
    let harness = Harness::new(&[], &[]);
    let fields = details(EVENT);
    enqueue_log_event(
        &harness.logger,
        &ScriptedRandom::new(&[0.5]),
        &fields,
        EventKind::Success,
    )
    .await;
    let queue = queue_snapshot(&harness.logger);
    assert_eq!(queue.len(), 1);
    assert_eq!(text(field(&queue[0], "id")), "call-1");
    assert!(harness.transport.calls().is_empty());
}

#[tokio::test]
async fn a_sampled_out_success_event_is_not_queued() {
    let harness = Harness::new(&[("RUBRIK_SAMPLING_RATE", "0")], &[]);
    let fields = details(EVENT);
    enqueue_log_event(
        &harness.logger,
        &ScriptedRandom::new(&[0.5]),
        &fields,
        EventKind::Success,
    )
    .await;
    assert!(queue_snapshot(&harness.logger).is_empty());
}

#[tokio::test]
async fn a_payload_error_is_logged_at_error_level() {
    let harness = Harness::new(&[], &[]);
    let fields = details(r#"{"litellm_call_id":"call-1"}"#);
    enqueue_log_event(
        &harness.logger,
        &ScriptedRandom::new(&[0.0]),
        &fields,
        EventKind::Failure,
    )
    .await;
    assert!(queue_snapshot(&harness.logger).is_empty());
    let errors = harness.logs.with_level(Level::ERROR);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "Rubrik logging hook failed. Skipping logging for this event."
    );
    assert_eq!(errors[0].event.as_deref(), Some("failure"));
}

#[tokio::test]
async fn reaching_the_batch_size_flushes_the_queue() {
    let harness = Harness::new(&[("RUBRIK_BATCH_SIZE", "1")], &[200]);
    let fields = details(EVENT);
    enqueue_log_event(
        &harness.logger,
        &ScriptedRandom::new(&[0.0]),
        &fields,
        EventKind::Success,
    )
    .await;
    assert!(queue_snapshot(&harness.logger).is_empty());
    let calls = harness.transport.calls();
    assert_eq!(calls.len(), 1);
    let Json::List(batch) = &calls[0] else {
        panic!("the flushed batch is a list");
    };
    assert_eq!(batch.len(), 1);
    assert_eq!(text(field(&batch[0], "id")), "call-1");
}

#[tokio::test]
async fn a_blocked_request_is_skipped_by_the_success_handler() {
    let harness = Harness::new(&[], &[]);
    let fields = details(
        r#"{"_rubrik_blocked":true,"litellm_call_id":"call-1","standard_logging_object":{"id":"chatcmpl-provider"}}"#,
    );
    async_log_success_event(&harness.logger, &UnusedRandom, &fields).await;
    assert!(queue_snapshot(&harness.logger).is_empty());
    let debug = harness.logs.with_level(Level::DEBUG);
    assert_eq!(debug.len(), 1);
    assert_eq!(
        debug[0].message,
        "Rubrik: skipping success event for blocked request"
    );
    assert_eq!(debug[0].litellm_call_id.as_deref(), Some("call-1"));
}

#[rstest]
#[case::absent(
    r#"{"litellm_call_id":"call-1","standard_logging_object":{"id":"chatcmpl-provider","status":"success"}}"#
)]
#[case::explicit_false(
    r#"{"_rubrik_blocked":false,"litellm_call_id":"call-1","standard_logging_object":{"id":"chatcmpl-provider","status":"success"}}"#
)]
#[case::null(
    r#"{"_rubrik_blocked":null,"litellm_call_id":"call-1","standard_logging_object":{"id":"chatcmpl-provider","status":"success"}}"#
)]
#[case::text(
    r#"{"_rubrik_blocked":"true","litellm_call_id":"call-1","standard_logging_object":{"id":"chatcmpl-provider","status":"success"}}"#
)]
#[tokio::test]
async fn a_non_true_block_flag_logs_the_success_event(#[case] call_details: &str) {
    let harness = Harness::new(&[], &[]);
    let fields = details(call_details);
    async_log_success_event(&harness.logger, &ScriptedRandom::new(&[0.0]), &fields).await;
    assert_eq!(queue_snapshot(&harness.logger).len(), 1);
}

#[tokio::test]
async fn a_blocked_request_still_logs_its_failure_event() {
    let harness = Harness::new(&[], &[]);
    let fields = details(
        r#"{"_rubrik_blocked":true,"litellm_call_id":"call-1","standard_logging_object":{"id":"chatcmpl-provider"}}"#,
    );
    enqueue_log_event(
        &harness.logger,
        &ScriptedRandom::new(&[0.0]),
        &fields,
        EventKind::Failure,
    )
    .await;
    let queue = queue_snapshot(&harness.logger);
    assert_eq!(queue.len(), 1);
    assert_eq!(text(field(&queue[0], "id")), "call-1");
}

#[tokio::test]
async fn a_sampled_out_failure_event_reports_the_failure_kind() {
    let harness = Harness::new(&[("RUBRIK_SAMPLING_RATE", "0")], &[]);
    let fields = details(EVENT);
    enqueue_log_event(
        &harness.logger,
        &ScriptedRandom::new(&[0.9]),
        &fields,
        EventKind::Failure,
    )
    .await;
    assert!(queue_snapshot(&harness.logger).is_empty());
    let debug = harness.logs.with_level(Level::DEBUG);
    assert_eq!(debug.len(), 1);
    assert_eq!(debug[0].event.as_deref(), Some("failure"));
}
