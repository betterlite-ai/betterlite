use betterlite_models::decisions::Json;

use super::frame_selection::assert_failed_message;
use super::support::{self, Answer, AnswerStep, Scenario};
use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_poll_state::store::decode_responses_poll_state;

const ITEM_ADDED: &str =
    r#"{"item":{"content":[],"id":"m1","type":"message"},"type":"response.output_item.added"}"#;
const PART_ADDED: &str = r#"{"item_id":"m1","part":{"text":"","type":"output_text"},"type":"response.content_part.added"}"#;
const COMPLETED: &str = r#"{"response":{"status":"completed"},"type":"response.completed"}"#;
// a record written after owners existed: the response sits inside the store's owner envelope
const ENVELOPE_SEED: &str = r#"{"owner":{"key_fingerprint":"k","team_id":"t","user_id":"u"},"response":{"created_at":100,"id":"litellm_poll_b936","metadata":{},"object":"response","output":[],"status":"queued"}}"#;

fn measured(values: Vec<Json>) -> usize {
    serde_json::to_vec(&Json::List(values))
        .expect("the measured values encode")
        .len()
}

fn encoded_len(record: &Json) -> usize {
    serde_json::to_vec(record)
        .expect("the record encodes")
        .len()
}

// the record the store holds once a patch lands: the seed response, with the status the worker wrote
fn response_record(status: &str, output: Json) -> Json {
    let Json::Object(mut seed) = support::seeded_response() else {
        panic!("the seed is an object");
    };
    seed.insert(String::from("status"), Json::Text(String::from(status)));
    seed.insert(String::from("output"), output);
    Json::Object(seed)
}

fn record_bytes(output: Json) -> usize {
    encoded_len(&response_record("in_progress", output))
}

fn scenario_for(events: &[&str], limits: ApiLimits) -> Scenario {
    let steps: Vec<AnswerStep> = support::frames(events)
        .into_iter()
        .map(AnswerStep::Chunk)
        .collect();
    support::scenario(&[1000], Answer::Streaming(steps)).with_limits(limits)
}

async fn run_with_limits(events: &[&str], limits: ApiLimits) -> Scenario {
    let mut scenario = scenario_for(events, limits);
    scenario
        .run()
        .await
        .expect("the worker records its outcome");
    scenario
}

// the store writes the owner envelope once B080 lands, so a seeded cache here holds one too
fn with_envelope(scenario: Scenario) -> Scenario {
    scenario.cache.seed(ENVELOPE_SEED);
    scenario
}

fn item(id: &str, extra: &str) -> String {
    format!(r#"{{"item":{{"content":[],"id":"{id}"{extra}}},"type":"response.output_item.added"}}"#)
}

fn assert_decodable(scenario: &Scenario, limits: ApiLimits) {
    let bytes = scenario.cache.last_write_bytes();
    match decode_responses_poll_state(&bytes, limits) {
        Ok(_state) => {}
        Err(error) => panic!("the cached record still decodes: {error:?}"),
    }
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
        "the request collected 17 items; the maximum is 10",
    );
    assert_decodable(&scenario, limits);
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
        "the request collected 21 items; the maximum is 12",
    );
    assert_decodable(&scenario, limits);
}

#[tokio::test]
async fn item_payloads_over_the_bound_fail_the_record() {
    let payload = "p".repeat(600);
    let first = item("m1", &format!(r#","payload":"{payload}""#));
    let second = item("m2", &format!(r#","payload":"{payload}""#));
    let output = support::json(&format!(
        r#"[{{"content":[],"id":"m1","payload":"{payload}"}},{{"content":[],"id":"m2","payload":"{payload}"}}]"#
    ));
    let observed = record_bytes(output);
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
    assert_decodable(&scenario, limits);
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
    let observed = record_bytes(output);
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
    assert_decodable(&scenario, limits);
}

// a delta for a content index with no part grows only the accumulated text, which the store never sees
#[tokio::test]
async fn deltas_without_a_part_past_the_json_bound_fail_the_record() {
    let text = "x".repeat(300);
    let delta = format!(
        r#"{{"content_index":0,"delta":"{text}","item_id":"m1","type":"response.output_text.delta"}}"#
    );
    let limits = ApiLimits {
        max_json_bytes: 500,
        max_event_bytes: 16 * 1024 * 1024,
        max_items: 64 * 1024,
    };
    let steps: Vec<AnswerStep> = support::frames(&[ITEM_ADDED, delta.as_str(), delta.as_str()])
        .into_iter()
        .map(AnswerStep::Chunk)
        .collect();
    let mut scenario =
        support::scenario(&[0, 0, 0, 200], Answer::Streaming(steps)).with_limits(limits);
    scenario
        .run()
        .await
        .expect("the worker records its outcome");
    // the due flush is refused before it writes, so only the status and the failed record land
    assert_eq!(scenario.cache.writes().len(), 2);
    assert_failed_message(
        &scenario,
        "the request JSON body is 604 bytes; the maximum is 500",
    );
    assert_decodable(&scenario, limits);
}

#[tokio::test]
async fn terminal_fields_over_the_bound_fail_the_record() {
    let instructions = "i".repeat(600);
    let completed = format!(
        r#"{{"response":{{"instructions":"{instructions}","status":"completed"}},"type":"response.completed"}}"#
    );
    let sized = run_with_limits(&[completed.as_str()], ApiLimits::default()).await;
    let observed = encoded_len(&support::last_write(&sized.cache));
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
    assert_decodable(&scenario, limits);
}

#[tokio::test]
async fn accumulated_bytes_over_the_event_bound_fail_the_record() {
    let payload = "p".repeat(300);
    let first = item("m1", &format!(r#","payload":"{payload}""#));
    let second = item("m2", &format!(r#","payload":"{payload}""#));
    let output = support::json(&format!(
        r#"[{{"content":[],"id":"m1","payload":"{payload}"}},{{"content":[],"id":"m2","payload":"{payload}"}}]"#
    ));
    let observed = record_bytes(output);
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
    assert_decodable(&scenario, limits);
}

#[tokio::test]
async fn flush_over_the_bound_fails_before_writing() {
    let payload = "p".repeat(600);
    let first = item("m1", &format!(r#","payload":"{payload}""#));
    let second = item("m2", &format!(r#","payload":"{payload}""#));
    let output = support::json(&format!(
        r#"[{{"content":[],"id":"m1","payload":"{payload}"}},{{"content":[],"id":"m2","payload":"{payload}"}}]"#
    ));
    let observed = record_bytes(output);
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
    assert_decodable(&scenario, limits);
}

#[tokio::test]
async fn a_limit_between_the_values_and_the_record_keeps_the_cache_decodable() {
    let payload = "p".repeat(600);
    let added = item("m1", &format!(r#","payload":"{payload}""#));
    let output = support::json(&format!(
        r#"[{{"content":[],"id":"m1","payload":"{payload}"}}]"#
    ));
    let mut sized = with_envelope(scenario_for(&[added.as_str()], ApiLimits::default()));
    sized.run().await.expect("the worker records its outcome");
    let flush = sized.cache.writes().into_iter().nth(1);
    let observed = encoded_len(&flush.expect("the flush record is written"));
    let limit = measured(vec![output]).saturating_add(1);
    assert!(limit < observed, "the limit sits below the record");
    let limits = ApiLimits {
        max_json_bytes: limit,
        max_event_bytes: 16 * 1024 * 1024,
        max_items: 64 * 1024,
    };
    let mut scenario = with_envelope(scenario_for(&[added.as_str()], limits));
    scenario
        .run()
        .await
        .expect("the worker records its outcome");
    assert_failed_message(
        &scenario,
        &format!("the request JSON body is {observed} bytes; the maximum is {limit}"),
    );
    assert_decodable(&scenario, limits);
}

#[tokio::test]
async fn a_failure_record_over_the_bound_is_not_written() {
    let initial = encoded_len(&response_record("in_progress", Json::List(Vec::new())));
    let limit = initial.saturating_add(10);
    let limits = ApiLimits {
        max_json_bytes: limit,
        max_event_bytes: 16 * 1024 * 1024,
        max_items: 64 * 1024,
    };
    let mut scenario =
        support::scenario(&[1000], Answer::Failure(String::from("boom"))).with_limits(limits);
    let Json::Object(mut seed) = support::seeded_response() else {
        panic!("the seed is an object");
    };
    seed.insert(String::from("status"), Json::Text(String::from("failed")));
    seed.insert(
        String::from("error"),
        support::json(
            r#"{"code":"background_streaming_error","message":"boom","type":"internal_error"}"#,
        ),
    );
    let observed = encoded_len(&Json::Object(seed));
    let error = scenario
        .run()
        .await
        .expect_err("the failed record does not fit");
    assert_eq!(
        error.to_string(),
        format!("the request JSON body is {observed} bytes; the maximum is {limit}")
    );
    assert_eq!(scenario.cache.writes().len(), 1);
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "in_progress");
    assert_decodable(&scenario, limits);
}

#[tokio::test]
async fn a_completed_stream_stays_within_the_bounds() {
    let limits = ApiLimits::default();
    let scenario = run_with_limits(&[ITEM_ADDED, PART_ADDED, COMPLETED], limits).await;
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "completed");
    assert_decodable(&scenario, limits);
}
