use betterlite_models::decisions::Json;

use super::frame_selection::assert_failed_message;
use super::support::{self, Answer, AnswerStep, Scenario};
use crate::api::api_boundaries::ApiLimits;

const ITEM_ADDED: &str =
    r#"{"item":{"content":[],"id":"m1","type":"message"},"type":"response.output_item.added"}"#;
const PART_ADDED: &str = r#"{"item_id":"m1","part":{"text":"","type":"output_text"},"type":"response.content_part.added"}"#;
const COMPLETED: &str = r#"{"response":{"status":"completed"},"type":"response.completed"}"#;

fn measured(values: Vec<Json>) -> usize {
    serde_json::to_vec(&Json::List(values))
        .expect("the measured values encode")
        .len()
}

fn with_status(output: Json) -> Vec<Json> {
    vec![output, Json::Text(String::from("completed"))]
}

async fn run_with_limits(events: &[&str], limits: ApiLimits) -> Scenario {
    let steps: Vec<AnswerStep> = support::frames(events)
        .into_iter()
        .map(AnswerStep::Chunk)
        .collect();
    let mut scenario = support::scenario(&[1000], Answer::Streaming(steps)).with_limits(limits);
    scenario
        .run()
        .await
        .expect("the worker records its outcome");
    scenario
}

fn item(id: &str, extra: &str) -> String {
    format!(r#"{{"item":{{"content":[],"id":"{id}"{extra}}},"type":"response.output_item.added"}}"#)
}

#[tokio::test]
async fn items_over_the_bound_fail_the_record() {
    let limits = ApiLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes: 1024 * 1024,
        max_items: 10,
    };
    let events = [
        ITEM_ADDED,
        r#"{"item":{"content":[],"id":"m2","type":"message"},"type":"response.output_item.added"}"#,
        r#"{"item":{"content":[],"id":"m3","type":"message"},"type":"response.output_item.added"}"#,
    ];
    let scenario = run_with_limits(&events, limits).await;
    assert_failed_message(
        &scenario,
        "the request collected 12 items; the maximum is 10",
    );
}

#[tokio::test]
async fn content_parts_over_the_bound_fail_the_record() {
    let limits = ApiLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes: 1024 * 1024,
        max_items: 12,
    };
    let events = [ITEM_ADDED, PART_ADDED, PART_ADDED, PART_ADDED, PART_ADDED];
    let scenario = run_with_limits(&events, limits).await;
    assert_failed_message(
        &scenario,
        "the request collected 16 items; the maximum is 12",
    );
}

#[tokio::test]
async fn item_payloads_over_the_bound_fail_the_record() {
    let payload = "p".repeat(600);
    let first = item("m1", &format!(r#","payload":"{payload}""#));
    let second = item("m2", &format!(r#","payload":"{payload}""#));
    let output = support::json(&format!(
        r#"[{{"content":[],"id":"m1","payload":"{payload}"}},{{"content":[],"id":"m2","payload":"{payload}"}}]"#
    ));
    let observed = measured(with_status(output));
    let limits = ApiLimits {
        max_json_bytes: 1024,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    let scenario = run_with_limits(&[first.as_str(), second.as_str()], limits).await;
    assert_failed_message(
        &scenario,
        &format!("the request JSON body is {observed} bytes; the maximum is 1024"),
    );
}

#[tokio::test]
async fn text_over_the_bound_fails_the_record() {
    let text = "x".repeat(600);
    let delta = format!(
        r#"{{"content_index":0,"delta":"{text}","item_id":"m1","type":"response.output_text.delta"}}"#
    );
    let output = support::json(&format!(
        r#"[{{"content":[{{"text":"{text}","type":"output_text"}}],"id":"m1","type":"message"}}]"#
    ));
    let observed = measured(with_status(output));
    let limit = observed.saturating_sub(1);
    let limits = ApiLimits {
        max_json_bytes: limit,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    let scenario = run_with_limits(&[ITEM_ADDED, PART_ADDED, delta.as_str()], limits).await;
    assert_failed_message(
        &scenario,
        &format!("the request JSON body is {observed} bytes; the maximum is {limit}"),
    );
}

#[tokio::test]
async fn terminal_fields_over_the_bound_fail_the_record() {
    let instructions = "i".repeat(600);
    let completed = format!(
        r#"{{"response":{{"instructions":"{instructions}","status":"completed"}},"type":"response.completed"}}"#
    );
    let values = vec![
        Json::List(Vec::new()),
        Json::Text(String::from("completed")),
        Json::Text(instructions),
    ];
    let observed = measured(values);
    let limits = ApiLimits {
        max_json_bytes: 512,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    let scenario = run_with_limits(&[completed.as_str()], limits).await;
    assert_failed_message(
        &scenario,
        &format!("the request JSON body is {observed} bytes; the maximum is 512"),
    );
}

#[tokio::test]
async fn accumulated_bytes_over_the_event_bound_fail_the_record() {
    let payload = "p".repeat(300);
    let first = item("m1", &format!(r#","payload":"{payload}""#));
    let second = item("m2", &format!(r#","payload":"{payload}""#));
    let output = support::json(&format!(
        r#"[{{"content":[],"id":"m1","payload":"{payload}"}},{{"content":[],"id":"m2","payload":"{payload}"}}]"#
    ));
    let observed = measured(with_status(output));
    let limits = ApiLimits {
        max_json_bytes: 16 * 1024 * 1024,
        max_event_bytes: 512,
        max_items: 64 * 1024,
    };
    let scenario = run_with_limits(&[first.as_str(), second.as_str()], limits).await;
    assert_failed_message(
        &scenario,
        &format!("the provider event is {observed} bytes; the maximum is 512"),
    );
}

#[tokio::test]
async fn flush_over_the_bound_fails_before_writing() {
    let payload = "p".repeat(600);
    let first = item("m1", &format!(r#","payload":"{payload}""#));
    let second = item("m2", &format!(r#","payload":"{payload}""#));
    let output = support::json(&format!(
        r#"[{{"content":[],"id":"m1","payload":"{payload}"}},{{"content":[],"id":"m2","payload":"{payload}"}}]"#
    ));
    let observed = measured(with_status(output));
    let limits = ApiLimits {
        max_json_bytes: 1024,
        max_event_bytes: 1024 * 1024,
        max_items: 64 * 1024,
    };
    let steps: Vec<AnswerStep> = support::frames(&[first.as_str(), second.as_str()])
        .into_iter()
        .map(AnswerStep::Chunk)
        .collect();
    let mut scenario =
        support::scenario(&[0, 0, 200], Answer::Streaming(steps)).with_limits(limits);
    scenario
        .run()
        .await
        .expect("the worker records its outcome");
    assert_eq!(scenario.cache.writes().len(), 2);
    assert_failed_message(
        &scenario,
        &format!("the request JSON body is {observed} bytes; the maximum is 1024"),
    );
}

#[tokio::test]
async fn a_completed_stream_stays_within_the_bounds() {
    let scenario =
        run_with_limits(&[ITEM_ADDED, PART_ADDED, COMPLETED], ApiLimits::default()).await;
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "completed");
}
