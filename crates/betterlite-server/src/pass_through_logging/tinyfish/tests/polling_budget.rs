#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use tokio::time::Instant;

use super::TINYFISH_POLL_SECONDS;
use super::billing::AsyncRun;
use super::poll::{MonotonicClock, PollClock, PollSleeper, TokioSleeper};
use super::support::{
    Fixture, ScriptedReply, capture_logs, dispatch_handle, fixture, json, kwargs,
    shared_logging_object, standard_text, start_time, unreachable,
};

const PENDING_BODY: &str = r#"{"run_id": "run-1", "status": "PENDING"}"#;
const COMPLETED_BODY: &str = r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3}"#;
const COMPLETED_TEXT: &str = r#"{"run_id":"run-1","status":"COMPLETED","num_of_steps":3}"#;

async fn poll_to_completion(fixture: &Fixture, logging_obj: Arc<StreamLoggingObject>) {
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
}

// The production clock reads wall time from the origin it was built with, which the fake clock in the other tests replaces.
#[tokio::test]
async fn the_monotonic_clock_reads_the_time_since_its_origin() {
    let origin = Instant::now();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let clock = MonotonicClock { origin };

    assert!(clock.now() >= Duration::from_millis(20));
}

// The production sleeper waits on the runtime clock, which the paused test clock moves without a real five seconds.
#[tokio::test(start_paused = true)]
async fn the_tokio_sleeper_waits_for_the_poll_interval() {
    let started = Instant::now();
    TokioSleeper.sleep(TINYFISH_POLL_SECONDS).await;

    assert!(started.elapsed() >= TINYFISH_POLL_SECONDS);
}

#[tokio::test]
async fn polls_every_five_seconds() {
    let fixture = fixture(vec![
        ScriptedReply::Run(200, PENDING_BODY),
        ScriptedReply::Run(200, COMPLETED_BODY),
    ]);
    poll_to_completion(&fixture, shared_logging_object()).await;

    assert_eq!(fixture.sleeper.slept(), vec![Duration::from_secs(5)]);
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(standard_text(&call.result), COMPLETED_TEXT);
}

#[tokio::test]
async fn stops_at_the_absolute_deadline() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Repeating(200, PENDING_BODY)]);
    poll_to_completion(&fixture, shared_logging_object()).await;

    assert_eq!(fixture.transport.requests().len(), 720);
    assert_eq!(fixture.sleeper.slept(), vec![Duration::from_secs(5); 720]);
    assert!(logs.contents().contains("not terminal after 3600s"));
    let calls = fixture.dispatch.calls();
    assert_eq!(
        standard_text(&calls.first().expect("one dispatch").result),
        r#"{"run_id":"run-1","status":"PENDING"}"#
    );
}

#[tokio::test]
async fn gives_up_after_twelve_consecutive_failures() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Unreachable]);
    poll_to_completion(&fixture, shared_logging_object()).await;

    assert_eq!(fixture.transport.requests().len(), 12);
    assert_eq!(fixture.sleeper.slept().len(), 11);
    assert!(
        logs.contents()
            .contains("giving up on run run-1 after 12 consecutive poll failures")
    );
    assert!(logs.contents().contains("logging the request without cost"));
    assert_eq!(fixture.dispatch.calls().len(), 1);
}

#[tokio::test]
async fn eleven_failures_do_not_stop_polling() {
    let mut replies = unreachable(11);
    replies.push(ScriptedReply::Run(200, COMPLETED_BODY));
    let fixture = fixture(replies);
    poll_to_completion(&fixture, shared_logging_object()).await;

    assert_eq!(fixture.transport.requests().len(), 12);
    assert_eq!(fixture.sleeper.slept().len(), 11);
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(standard_text(&call.result), COMPLETED_TEXT);
}

// Without the reset the eleven failures before the success would carry over and end the poll one failure later.
#[tokio::test]
async fn a_success_resets_the_failure_count() {
    let (logs, _guard) = capture_logs();
    let mut replies = unreachable(11);
    replies.push(ScriptedReply::Run(200, PENDING_BODY));
    replies.extend(unreachable(12));
    let fixture = fixture(replies);
    poll_to_completion(&fixture, shared_logging_object()).await;

    assert_eq!(fixture.transport.requests().len(), 24);
    assert!(
        logs.contents()
            .contains("giving up on run run-1 after 12 consecutive poll failures")
    );
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(
        standard_text(&call.result),
        r#"{"run_id":"run-1","status":"PENDING"}"#
    );
}

// A failed poll after a good one must not forget the run, so the give-up still bills the last run fetched.
#[tokio::test]
async fn keeps_the_last_run_across_failed_polls() {
    let (logs, _guard) = capture_logs();
    let mut replies = vec![ScriptedReply::Run(200, PENDING_BODY)];
    replies.extend(unreachable(12));
    let fixture = fixture(replies);
    poll_to_completion(&fixture, shared_logging_object()).await;

    assert_eq!(fixture.transport.requests().len(), 13);
    assert!(
        logs.contents()
            .contains("giving up on run run-1 after 12 consecutive poll failures")
    );
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(
        standard_text(&call.result),
        r#"{"run_id":"run-1","status":"PENDING"}"#
    );
}
