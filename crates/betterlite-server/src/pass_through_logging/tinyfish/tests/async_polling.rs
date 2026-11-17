#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use chrono::DateTime;

use super::TINYFISH_MODEL_NAME;
use super::billing::AsyncRun;
use super::support::{
    ScriptedReply, dispatch_handle, fixture, fixture_with_credentials, json, kwargs,
    shared_logging_object, standard_text, start_time,
};

const PENDING_BODY: &str = r#"{"run_id": "run-1", "status": "PENDING"}"#;
const COMPLETED_BODY: &str = r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3}"#;
const COMPLETED_TEXT: &str = r#"{"run_id":"run-1","status":"COMPLETED","num_of_steps":3}"#;

#[tokio::test]
async fn spawns_a_poller_that_bills_the_terminal_run() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = shared_logging_object();
    let body = json(PENDING_BODY);
    let handle = fixture.billing.start_async_run_billing(AsyncRun {
        logging_obj: Arc::clone(&logging_obj),
        response_body: Some(&body),
        result: "pending text".to_owned(),
        kwargs: kwargs(&[]),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });
    handle.await.expect("the poller finishes");

    let requests = fixture.transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests.first().expect("one request").url,
        "https://agent.tinyfish.ai/v1/runs/run-1?screenshots=none"
    );
    let calls = fixture.dispatch.calls();
    assert_eq!(calls.len(), 1);
    let call = calls.first().expect("one dispatch");
    assert_eq!(call.model, TINYFISH_MODEL_NAME);
    assert_eq!(call.provider.as_deref(), Some("tinyfish"));
    assert_eq!(standard_text(&call.result), COMPLETED_TEXT);
    assert_eq!(
        call.kwargs.entries.get("response_cost"),
        Some(&json("0.048"))
    );
    assert_eq!(
        call.kwargs.entries.get("litellm_call_id"),
        Some(&Json::Text("call-1".to_owned()))
    );
    assert_eq!(call.start_time, start_time());
    assert_eq!(
        call.end_time,
        DateTime::parse_from_rfc3339("2024-01-01T00:00:05+00:00").expect("a valid timestamp")
    );
}

#[tokio::test]
async fn polls_through_a_pending_run_before_it_bills() {
    let fixture = fixture(vec![
        ScriptedReply::Run(200, PENDING_BODY),
        ScriptedReply::Run(200, COMPLETED_BODY),
    ]);
    let logging_obj = shared_logging_object();
    let body = json(PENDING_BODY);
    let handle = fixture.billing.start_async_run_billing(AsyncRun {
        logging_obj,
        response_body: Some(&body),
        result: String::new(),
        kwargs: kwargs(&[]),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });
    handle.await.expect("the poller finishes");

    assert_eq!(fixture.transport.requests().len(), 2);
    assert_eq!(fixture.sleeper.slept(), vec![Duration::from_secs(5)]);
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(standard_text(&call.result), COMPLETED_TEXT);
}

// A single dead poll must not lose the charge of the run that later completes.
#[tokio::test]
async fn a_failed_fetch_does_not_lose_the_later_terminal_run() {
    let fixture = fixture(vec![
        ScriptedReply::Unreachable,
        ScriptedReply::Run(200, COMPLETED_BODY),
    ]);
    let logging_obj = shared_logging_object();
    let body = json(PENDING_BODY);
    let handle = fixture.billing.start_async_run_billing(AsyncRun {
        logging_obj,
        response_body: Some(&body),
        result: String::new(),
        kwargs: kwargs(&[]),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });
    handle.await.expect("the poller finishes");

    assert_eq!(fixture.transport.requests().len(), 2);
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(
        call.kwargs.entries.get("response_cost"),
        Some(&json("0.048"))
    );
}

#[tokio::test]
async fn the_last_pending_run_is_logged_without_cost_at_the_deadline() {
    let fixture = fixture(vec![ScriptedReply::Repeating(200, PENDING_BODY)]);
    let logging_obj = shared_logging_object();
    let body = json(PENDING_BODY);
    let handle = fixture.billing.start_async_run_billing(AsyncRun {
        logging_obj,
        response_body: Some(&body),
        result: String::new(),
        kwargs: kwargs(&[]),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });
    handle.await.expect("the poller finishes");

    assert_eq!(fixture.transport.requests().len(), 720);
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(
        standard_text(&call.result),
        r#"{"run_id":"run-1","status":"PENDING"}"#
    );
    assert_eq!(call.kwargs.entries.get("response_cost"), Some(&Json::Null));
}

// With no run to poll, the request is still logged and the upstream text is all the payload has.
#[tokio::test]
async fn the_written_result_is_logged_when_no_run_is_fetched() {
    let fixture = fixture_with_credentials(Vec::new(), None);
    let logging_obj = shared_logging_object();
    let body = json(PENDING_BODY);
    let handle = fixture.billing.start_async_run_billing(AsyncRun {
        logging_obj,
        response_body: Some(&body),
        result: "upstream text".to_owned(),
        kwargs: kwargs(&[]),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });
    handle.await.expect("the poller finishes");

    assert!(fixture.transport.requests().is_empty());
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(standard_text(&call.result), "upstream text");
    assert_eq!(call.kwargs.entries.get("response_cost"), Some(&Json::Null));
    assert_eq!(call.model, TINYFISH_MODEL_NAME);
}
