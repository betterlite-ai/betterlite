#![cfg(test)]

use rstest::rstest;

use super::billing::AsyncRun;
use super::fetch::{fetch_run, safe_run_id};
use super::support::{
    FixedCredentials, ScriptedReply, capture_logs, dispatch_handle, fixture, json, kwargs, secret,
    shared_logging_object, start_time,
};

const COMPLETED_BODY: &str = r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3}"#;
const API_BASE: &str = "https://agent.tinyfish.ai";

#[rstest]
#[case::a_slash("a/b", None)]
#[case::a_backslash("a\\b", None)]
#[case::a_hash("a#b", None)]
#[case::a_question_mark("a?b", None)]
#[case::the_parent_segment("..", None)]
#[case::an_embedded_parent("a..b", None)]
#[case::a_plain_id("run-1", Some("run-1"))]
#[case::a_single_dot("a.b", Some("a.b"))]
#[case::a_space("a b", Some("a%20b"))]
fn rejects_ids_that_would_leave_the_segment(#[case] run_id: &str, #[case] expected: Option<&str>) {
    assert_eq!(safe_run_id(run_id), expected.map(ToOwned::to_owned));
}

#[tokio::test]
async fn an_invalid_run_id_warns_and_makes_no_request() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let credentials = FixedCredentials {
        key: Some(secret("key-1")),
    };
    let run = fetch_run(
        &credentials,
        fixture.transport.as_ref(),
        API_BASE,
        "../admin",
    )
    .await;

    assert!(run.is_none());
    assert!(fixture.transport.requests().is_empty());
    assert!(logs.contents().contains("invalid run_id"));
}

#[tokio::test]
async fn an_invalid_run_id_counts_as_a_failed_poll() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Repeating(200, COMPLETED_BODY)]);
    let body = json(r#"{"run_id": "run/1", "status": "PENDING"}"#);
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
    assert!(
        logs.contents()
            .contains("giving up on run run/1 after 12 consecutive poll failures")
    );
    assert_eq!(fixture.dispatch.calls().len(), 1);
}
