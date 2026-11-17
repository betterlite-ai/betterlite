#![cfg(test)]

use std::time::Duration;

use rstest::rstest;

use super::fetch::{RUN_API_KEY_HEADER, RUN_FETCH_TIMEOUT, fetch_run};
use super::support::{FixedCredentials, ScriptedReply, capture_logs, fixture, secret};

const COMPLETED_BODY: &str = r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3}"#;
const API_BASE: &str = "https://agent.tinyfish.ai";

#[tokio::test]
async fn sends_the_key_and_the_screenshots_query() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let credentials = FixedCredentials {
        key: Some(secret("key-1")),
    };
    let run = fetch_run(&credentials, fixture.transport.as_ref(), API_BASE, "run-1").await;

    assert!(run.is_some());
    let requests = fixture.transport.requests();
    let request = requests.first().expect("one request");
    assert_eq!(
        request.url,
        "https://agent.tinyfish.ai/v1/runs/run-1?screenshots=none"
    );
    assert_eq!(request.api_key.as_deref(), Some("key-1"));
    assert_eq!(request.timeout, RUN_FETCH_TIMEOUT);
}

// LiteLLM sends the provider key as X-API-Key on a poll client it builds with a 30-second timeout.
#[test]
fn names_the_key_header_and_the_poll_timeout() {
    assert_eq!(RUN_API_KEY_HEADER, "x-api-key");
    assert_eq!(RUN_FETCH_TIMEOUT, Duration::from_secs(30));
}

#[tokio::test]
async fn a_missing_key_fetches_nothing() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let credentials = FixedCredentials { key: None };
    let run = fetch_run(&credentials, fixture.transport.as_ref(), API_BASE, "run-1").await;

    assert!(run.is_none());
    assert!(fixture.transport.requests().is_empty());
    assert!(
        logs.contents()
            .contains("no API key available to poll run run-1")
    );
}

#[rstest]
#[case::a_space("run id", "run%20id")]
#[case::a_plus("run+id", "run%2Bid")]
#[case::a_colon("run:id", "run%3Aid")]
#[tokio::test]
async fn percent_encodes_the_run_id(#[case] run_id: &str, #[case] encoded: &str) {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let credentials = FixedCredentials {
        key: Some(secret("key-1")),
    };
    let run = fetch_run(&credentials, fixture.transport.as_ref(), API_BASE, run_id).await;

    assert!(run.is_some());
    assert_eq!(
        fixture
            .transport
            .requests()
            .first()
            .expect("one request")
            .url,
        format!("https://agent.tinyfish.ai/v1/runs/{encoded}?screenshots=none")
    );
}

#[rstest]
#[case::ok(200, true)]
#[case::the_last_ok(299, true)]
#[case::a_redirect(300, false)]
#[case::a_client_error(404, false)]
#[case::a_server_error(500, false)]
#[tokio::test]
async fn only_a_2xx_answer_carries_a_run(#[case] status: u16, #[case] expected: bool) {
    let fixture = fixture(vec![ScriptedReply::Run(status, COMPLETED_BODY)]);
    let credentials = FixedCredentials {
        key: Some(secret("key-1")),
    };
    let run = fetch_run(&credentials, fixture.transport.as_ref(), API_BASE, "run-1").await;

    assert_eq!(run.is_some(), expected);
}

#[tokio::test]
async fn reads_the_run_from_the_answer() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let credentials = FixedCredentials {
        key: Some(secret("key-1")),
    };
    let run = fetch_run(&credentials, fixture.transport.as_ref(), API_BASE, "run-1")
        .await
        .expect("the run parses");

    assert_eq!(run.run_id.flatten().as_deref(), Some("run-1"));
    assert_eq!(run.status.flatten().as_deref(), Some("COMPLETED"));
    assert_eq!(run.num_of_steps.flatten(), Some(3));
}

#[tokio::test]
async fn a_body_that_is_not_json_is_a_failed_poll() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Run(200, "not json")]);
    let credentials = FixedCredentials {
        key: Some(secret("key-1")),
    };
    let run = fetch_run(&credentials, fixture.transport.as_ref(), API_BASE, "run-1").await;

    assert!(run.is_none());
    assert!(
        logs.contents()
            .contains("[Non blocking logging error] TinyFish run fetch failed")
    );
}

#[tokio::test]
async fn a_body_that_is_not_utf8_is_a_failed_poll() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Bytes(200, b"\xff\xfe")]);
    let credentials = FixedCredentials {
        key: Some(secret("key-1")),
    };
    let run = fetch_run(&credentials, fixture.transport.as_ref(), API_BASE, "run-1").await;

    assert!(run.is_none());
    assert!(
        logs.contents()
            .contains("[Non blocking logging error] TinyFish run fetch failed")
    );
}

#[tokio::test]
async fn a_body_of_the_wrong_shape_is_a_failed_poll() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Run(200, "[1, 2]")]);
    let credentials = FixedCredentials {
        key: Some(secret("key-1")),
    };
    let run = fetch_run(&credentials, fixture.transport.as_ref(), API_BASE, "run-1").await;

    assert!(run.is_none());
    assert!(logs.contents().contains("unexpected run object shape"));
}

#[tokio::test]
async fn a_transport_error_is_a_failed_poll() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Unreachable]);
    let credentials = FixedCredentials {
        key: Some(secret("key-1")),
    };
    let run = fetch_run(&credentials, fixture.transport.as_ref(), API_BASE, "run-1").await;

    assert!(run.is_none());
    assert_eq!(fixture.transport.requests().len(), 1);
    assert!(
        logs.contents()
            .contains("[Non blocking logging error] TinyFish run fetch failed")
    );
}
