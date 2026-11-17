#![cfg(test)]

use std::sync::Arc;
use std::sync::PoisonError;

use betterlite_models::decisions::Json;

use super::billing::{AsyncRun, BlockingRun, CollectedChunks};
use super::support::{
    FailingPayload, ScriptedReply, capture_logs, dispatch_handle, fixture,
    fixture_with_credentials, fixture_with_payload, json, kwargs, logging_object, outcome_text,
    secret, shared_logging_object, standard_text, start_time,
};

const PENDING_BODY: &str = r#"{"run_id": "run-1", "status": "PENDING"}"#;
const COMPLETED_BODY: &str = r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3}"#;

#[tokio::test]
async fn a_failed_fetch_still_dispatches_the_request_without_cost() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Unreachable]);
    let body = json(PENDING_BODY);
    fixture
        .billing
        .start_async_run_billing(AsyncRun {
            logging_obj: shared_logging_object(),
            response_body: Some(&body),
            result: String::new(),
            kwargs: kwargs(&[]),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    assert_eq!(fixture.dispatch.calls().len(), 1);
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(call.kwargs.entries.get("response_cost"), Some(&Json::Null));
    assert!(
        logs.contents()
            .contains("[Non blocking logging error] TinyFish run fetch failed")
    );
}

#[tokio::test]
async fn the_deadline_still_bills_the_last_run_without_cost() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Repeating(200, PENDING_BODY)]);
    let body = json(PENDING_BODY);
    fixture
        .billing
        .start_async_run_billing(AsyncRun {
            logging_obj: shared_logging_object(),
            response_body: Some(&body),
            result: String::new(),
            kwargs: kwargs(&[]),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    assert_eq!(fixture.dispatch.calls().len(), 1);
    assert!(logs.contents().contains("not terminal after 3600s"));
    assert!(logs.contents().contains("logging the request without cost"));
}

#[test]
fn the_blocking_handler_keeps_the_raw_result_when_the_payload_fails() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture_with_payload(
        vec![ScriptedReply::Run(200, COMPLETED_BODY)],
        Arc::new(FailingPayload),
    );
    let logging_obj = logging_object();
    let body = json(COMPLETED_BODY);
    let caller = kwargs(&[("team", Json::Text("team-1".to_owned()))]);
    let outcome = fixture.billing.tinyfish_passthrough_handler(BlockingRun {
        logging_obj: &logging_obj,
        response_body: Some(&body),
        result: "raw upstream text",
        kwargs: &caller,
    });

    assert_eq!(outcome_text(&outcome), "raw upstream text");
    assert_eq!(outcome.kwargs, caller);
    assert!(
        logs.contents()
            .contains("Error in TinyFish passthrough logging handler")
    );
}

#[tokio::test]
async fn the_sse_fallback_answers_empty_when_the_payload_fails() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture_with_payload(
        vec![ScriptedReply::Run(200, COMPLETED_BODY)],
        Arc::new(FailingPayload),
    );
    let logging_obj = logging_object();
    let lines = vec![String::from("data: {\"run_id\": \"run-9\"}")];
    let outcome = fixture
        .billing
        .handle_logging_tinyfish_collected_chunks(CollectedChunks {
            logging_obj: &logging_obj,
            lines: &lines,
        })
        .await;

    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(outcome_text(&outcome), "");
    assert!(outcome.kwargs.entries.is_empty());
    assert!(
        logs.contents()
            .contains("Error in TinyFish SSE passthrough logging handler")
    );
}

#[tokio::test]
async fn the_poller_skips_the_dispatch_when_the_payload_fails() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture_with_payload(
        vec![ScriptedReply::Run(200, COMPLETED_BODY)],
        Arc::new(FailingPayload),
    );
    let body = json(PENDING_BODY);
    fixture
        .billing
        .start_async_run_billing(AsyncRun {
            logging_obj: shared_logging_object(),
            response_body: Some(&body),
            result: String::new(),
            kwargs: kwargs(&[]),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    assert!(fixture.dispatch.calls().is_empty());
    assert!(
        logs.contents()
            .contains("[Non blocking logging error] TinyFish run-async billing failed")
    );
}

#[tokio::test]
async fn a_taken_logging_state_is_logged_and_nothing_is_dispatched() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = shared_logging_object();
    drop(
        logging_obj
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(),
    );
    let body = json(PENDING_BODY);
    fixture
        .billing
        .start_async_run_billing(AsyncRun {
            logging_obj,
            response_body: Some(&body),
            result: String::new(),
            kwargs: kwargs(&[]),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    assert!(fixture.dispatch.calls().is_empty());
    assert!(
        logs.contents()
            .contains("the TinyFish logging state was taken before the billing dispatch")
    );
}

#[tokio::test]
async fn a_key_that_cannot_be_a_header_fails_the_poll_without_panicking() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture_with_credentials(
        vec![ScriptedReply::Repeating(200, COMPLETED_BODY)],
        Some(secret("bad\nkey")),
    );
    let body = json(PENDING_BODY);
    fixture
        .billing
        .start_async_run_billing(AsyncRun {
            logging_obj: shared_logging_object(),
            response_body: Some(&body),
            result: String::new(),
            kwargs: kwargs(&[]),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    assert!(fixture.transport.requests().is_empty());
    assert_eq!(fixture.dispatch.calls().len(), 1);
    let calls = fixture.dispatch.calls();
    assert_eq!(
        standard_text(&calls.first().expect("one dispatch").result),
        ""
    );
    assert!(
        logs.contents()
            .contains("[Non blocking logging error] TinyFish run fetch failed")
    );
}
