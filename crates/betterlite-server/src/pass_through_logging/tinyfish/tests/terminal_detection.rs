#![cfg(test)]

use rstest::rstest;

use super::billing::AsyncRun;
use super::poll::is_terminal;
use super::support::{
    ScriptedReply, dispatch_handle, fixture, json, kwargs, run, shared_logging_object,
    standard_text, start_time,
};

const PENDING_BODY: &str = r#"{"run_id": "run-1", "status": "PENDING"}"#;
const COMPLETED_BODY: &str = r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3}"#;
const FAILED_BODY: &str = r#"{"run_id": "run-1", "status": "FAILED", "num_of_steps": 3}"#;

#[rstest]
#[case::completed(r#"{"status": "COMPLETED"}"#, true)]
#[case::failed(r#"{"status": "FAILED"}"#, true)]
#[case::cancelled(r#"{"status": "CANCELLED"}"#, true)]
#[case::running(r#"{"status": "RUNNING"}"#, false)]
#[case::pending(r#"{"status": "PENDING"}"#, false)]
#[case::lowercase(r#"{"status": "completed"}"#, false)]
#[case::absent_status("{}", false)]
#[case::null_status(r#"{"status": null}"#, false)]
#[case::empty_status(r#"{"status": ""}"#, false)]
#[case::padded_status(r#"{"status": " COMPLETED "}"#, false)]
fn reads_the_terminal_status(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(is_terminal(&run(text)), expected);
}

#[tokio::test]
async fn a_terminal_first_poll_returns_at_once() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
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

    assert_eq!(fixture.transport.requests().len(), 1);
    assert!(fixture.sleeper.slept().is_empty());
}

#[tokio::test]
async fn a_non_terminal_run_keeps_polling() {
    let fixture = fixture(vec![
        ScriptedReply::Run(200, PENDING_BODY),
        ScriptedReply::Run(200, FAILED_BODY),
    ]);
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

    assert_eq!(fixture.transport.requests().len(), 2);
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(
        standard_text(&call.result),
        r#"{"run_id":"run-1","status":"FAILED","num_of_steps":3}"#
    );
}

#[tokio::test]
async fn a_run_without_a_status_keeps_polling() {
    let fixture = fixture(vec![
        ScriptedReply::Run(200, "{}"),
        ScriptedReply::Run(200, COMPLETED_BODY),
    ]);
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

    assert_eq!(fixture.transport.requests().len(), 2);
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(
        call.kwargs.entries.get("response_cost"),
        Some(&json("0.048"))
    );
}
