#![cfg(test)]

use betterlite_models::decisions::Json;

use super::billing::CollectedChunks;
use super::support::{
    ScriptedReply, capture_logs, detail_cost, fixture, json, logged_provider, logging_object,
    outcome_text,
};

const COMPLETED_BODY: &str = r#"{"run_id": "run-9", "status": "COMPLETED", "num_of_steps": 3}"#;
const PENDING_BODY: &str = r#"{"run_id": "run-9", "status": "PENDING"}"#;

fn chunks() -> Vec<String> {
    vec![
        String::from("event: message"),
        String::from("data: {\"run_id\": \"run-9\"}"),
    ]
}

#[tokio::test]
async fn one_fetch_prices_the_run_found_in_the_chunks() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = logging_object();
    let lines = chunks();
    let outcome = fixture
        .billing
        .handle_logging_tinyfish_collected_chunks(CollectedChunks {
            logging_obj: &logging_obj,
            lines: &lines,
        })
        .await;

    let requests = fixture.transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests.first().expect("one request").url,
        "https://agent.tinyfish.ai/v1/runs/run-9?screenshots=none"
    );
    assert_eq!(
        outcome_text(&outcome),
        r#"{"run_id":"run-9","status":"COMPLETED","num_of_steps":3}"#
    );
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&json("0.048"))
    );
    assert_eq!(detail_cost(&logging_obj).as_deref(), Some("0.048"));
    assert_eq!(logged_provider(&logging_obj).as_deref(), Some("tinyfish"));
}

// The fallback asks once, so a pending answer ends the pricing rather than starting a poll loop.
#[tokio::test]
async fn a_pending_answer_is_fetched_only_once() {
    let fixture = fixture(vec![ScriptedReply::Repeating(200, PENDING_BODY)]);
    let logging_obj = logging_object();
    let lines = chunks();
    let outcome = fixture
        .billing
        .handle_logging_tinyfish_collected_chunks(CollectedChunks {
            logging_obj: &logging_obj,
            lines: &lines,
        })
        .await;

    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(
        outcome_text(&outcome),
        r#"{"run_id":"run-9","status":"PENDING"}"#
    );
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&Json::Null)
    );
}

#[tokio::test]
async fn a_failed_fetch_logs_the_request_without_cost() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(vec![ScriptedReply::Unreachable]);
    let logging_obj = logging_object();
    let lines = chunks();
    let outcome = fixture
        .billing
        .handle_logging_tinyfish_collected_chunks(CollectedChunks {
            logging_obj: &logging_obj,
            lines: &lines,
        })
        .await;

    assert_eq!(fixture.transport.requests().len(), 1);
    assert_eq!(outcome_text(&outcome), "");
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&Json::Null)
    );
    assert!(
        logs.contents()
            .contains("[Non blocking logging error] TinyFish run fetch failed")
    );
}

// B3648's parser skips a null run_id, so the next frame supplies the id and the fetch.
#[tokio::test]
async fn the_first_parseable_frame_supplies_the_run_id() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = logging_object();
    let lines = vec![
        String::from("data: {\"run_id\": null}"),
        String::from("data: not json"),
        String::from("data: {\"run_id\": \"run-9\"}"),
    ];
    let outcome = fixture
        .billing
        .handle_logging_tinyfish_collected_chunks(CollectedChunks {
            logging_obj: &logging_obj,
            lines: &lines,
        })
        .await;

    assert_eq!(
        fixture
            .transport
            .requests()
            .first()
            .expect("one request")
            .url,
        "https://agent.tinyfish.ai/v1/runs/run-9?screenshots=none"
    );
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&json("0.048"))
    );
}

#[tokio::test]
async fn an_empty_stream_answers_empty_without_a_request() {
    let fixture = fixture(vec![ScriptedReply::Run(200, COMPLETED_BODY)]);
    let logging_obj = logging_object();
    let lines: Vec<String> = Vec::new();
    let outcome = fixture
        .billing
        .handle_logging_tinyfish_collected_chunks(CollectedChunks {
            logging_obj: &logging_obj,
            lines: &lines,
        })
        .await;

    assert!(fixture.transport.requests().is_empty());
    assert_eq!(outcome_text(&outcome), "");
    assert_eq!(
        outcome.kwargs.entries.get("response_cost"),
        Some(&Json::Null)
    );
}
