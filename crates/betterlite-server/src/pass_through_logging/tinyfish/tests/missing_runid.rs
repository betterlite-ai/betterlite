#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::billing::{AsyncRun, CollectedChunks, SseRun};
use super::sse::sse_poller_spawned;
use super::support::{
    ScriptedReply, capture_logs, dispatch_handle, fixture, json, kwargs, logging_object,
    outcome_text, shared_logging_object, standard_text, start_time,
};

const COMPLETED_BODY: &str = r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3}"#;

#[rstest]
#[case::no_run_id(r#"{"status": "PENDING"}"#)]
#[case::an_empty_run_id(r#"{"run_id": ""}"#)]
#[case::a_null_run_id(r#"{"run_id": null}"#)]
#[case::an_empty_body("{}")]
#[tokio::test]
async fn a_run_async_body_without_a_run_id_warns_and_makes_no_request(#[case] text: &str) {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let body = json(text);
    fixture
        .billing
        .start_async_run_billing(AsyncRun {
            logging_obj: shared_logging_object(),
            response_body: Some(&body),
            result: "written".to_owned(),
            kwargs: kwargs(&[]),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    assert!(fixture.transport.requests().is_empty());
    assert!(logs.contents().contains("carried no run_id"));
    assert!(logs.contents().contains("logging the request without cost"));
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(standard_text(&call.result), "written");
    assert_eq!(call.kwargs.entries.get("response_cost"), Some(&Json::Null));
}

#[tokio::test]
async fn a_response_body_that_is_absent_warns_and_makes_no_request() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    fixture
        .billing
        .start_async_run_billing(AsyncRun {
            logging_obj: shared_logging_object(),
            response_body: None,
            result: "written".to_owned(),
            kwargs: kwargs(&[]),
            log: dispatch_handle(&fixture.dispatch),
            start_time: start_time(),
        })
        .await
        .expect("the poller finishes");

    assert!(fixture.transport.requests().is_empty());
    assert!(logs.contents().contains("carried no run_id"));
    assert_eq!(fixture.dispatch.calls().len(), 1);
}

#[tokio::test]
async fn the_sse_fallback_without_a_run_id_warns_and_makes_no_request() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = logging_object();
    let lines = vec![
        String::from("event: message"),
        String::from("data: {\"status\": \"PENDING\"}"),
        String::from("data: not json"),
    ];
    let outcome = fixture
        .billing
        .handle_logging_tinyfish_collected_chunks(CollectedChunks {
            logging_obj: &logging_obj,
            lines: &lines,
        })
        .await;

    assert!(fixture.transport.requests().is_empty());
    assert!(logs.contents().contains("no run_id in SSE stream"));
    assert_eq!(outcome_text(&outcome), "");
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&Json::Null)
    );
}

#[tokio::test]
async fn an_sse_run_with_an_empty_run_id_makes_no_request() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = shared_logging_object();
    let handle = fixture.billing.start_sse_run_billing(SseRun {
        run_id: String::new(),
        logging_obj: Arc::clone(&logging_obj),
        log: dispatch_handle(&fixture.dispatch),
        start_time: start_time(),
    });
    handle.await.expect("the poller finishes");

    assert!(fixture.transport.requests().is_empty());
    assert!(sse_poller_spawned(&logging_obj));
    let calls = fixture.dispatch.calls();
    let call = calls.first().expect("one dispatch");
    assert_eq!(standard_text(&call.result), "");
    assert_eq!(call.kwargs.entries.get("response_cost"), Some(&Json::Null));
}
