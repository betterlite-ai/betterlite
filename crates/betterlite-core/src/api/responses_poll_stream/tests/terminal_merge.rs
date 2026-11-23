use betterlite_models::decisions::Json;

use super::support::{self, Answer, AnswerStep, Scenario};

pub(super) const ITEM_ADDED: &str =
    r#"{"item":{"content":[],"id":"m1","type":"message"},"type":"response.output_item.added"}"#;
pub(super) const M2_ADDED: &str =
    r#"{"item":{"content":[],"id":"m2","type":"message"},"type":"response.output_item.added"}"#;
const PART_ADDED: &str = r#"{"item_id":"m1","part":{"text":"","type":"output_text"},"type":"response.content_part.added"}"#;
const DELTA: &str =
    r#"{"content_index":0,"delta":"A","item_id":"m1","type":"response.output_text.delta"}"#;
const COMPLETED_WITH_M2: &str = r#"{"response":{"output":[{"content":[],"id":"m2","type":"message"}],"status":"completed"},"type":"response.completed"}"#;
const COMPLETED_EMPTY: &str =
    r#"{"response":{"output":[],"status":"completed"},"type":"response.completed"}"#;

pub(super) async fn with_events(events: &[&str]) -> Scenario {
    let steps: Vec<AnswerStep> = support::frames(events)
        .into_iter()
        .map(AnswerStep::Chunk)
        .collect();
    let mut scenario = support::scenario(&[1000], Answer::Streaming(steps));
    scenario.run().await.expect("the worker completes");
    scenario
}

pub(super) async fn merged_items() -> Scenario {
    with_events(&[ITEM_ADDED, PART_ADDED, DELTA, COMPLETED_WITH_M2]).await
}

pub(super) async fn terminal_keeps_iterating() -> Scenario {
    with_events(&[ITEM_ADDED, PART_ADDED, DELTA, COMPLETED_EMPTY, M2_ADDED]).await
}

pub(super) async fn empty_terminal_output() -> Scenario {
    with_events(&[ITEM_ADDED, PART_ADDED, DELTA, COMPLETED_EMPTY]).await
}

pub(super) async fn failure_terminal() -> Scenario {
    let failed = r#"{"response":{"error":{"code":"x","message":"down"},"status":"failed"},"type":"response.failed"}"#;
    with_events(&[ITEM_ADDED, failed]).await
}

pub(super) async fn terminal_fields() -> Scenario {
    let completed = r#"{"response":{"incomplete_details":{},"parallel_tool_calls":false,"status":"completed","temperature":0,"tools":[],"usage":{"input_tokens":0}},"type":"response.completed"}"#;
    with_events(&[ITEM_ADDED, completed]).await
}

pub(super) async fn non_streaming() -> (Scenario, Vec<String>) {
    let (guard, logs) = support::capture_logs();
    let mut scenario = support::scenario(&[1000], Answer::NonStreaming);
    scenario.run().await.expect("the worker completes");
    drop(guard);
    (scenario, logs.messages("WARN"))
}

pub(super) fn output_ids(record: &Json) -> Vec<String> {
    let Json::List(items) = support::field(record, "output") else {
        panic!("the output is a list");
    };
    items.iter().map(|item| support::text(item, "id")).collect()
}

pub(super) fn assert_m1_then_m2(scenario: &Scenario) {
    let record = support::last_write(&scenario.cache);
    assert_eq!(
        output_ids(&record),
        vec![String::from("m1"), String::from("m2")]
    );
    assert_eq!(support::first_text(&record), "A");
}

pub(super) fn assert_only_m1(scenario: &Scenario) {
    let record = support::last_write(&scenario.cache);
    assert_eq!(output_ids(&record), vec![String::from("m1")]);
    assert_eq!(support::first_text(&record), "A");
}

pub(super) fn assert_failure_captured(scenario: &Scenario) {
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "failed");
    assert_eq!(
        support::field(&record, "error"),
        support::json(r#"{"code":"x","message":"down"}"#)
    );
}

pub(super) fn assert_terminal_fields(scenario: &Scenario) {
    let record = support::last_write(&scenario.cache);
    assert_eq!(
        support::field(&record, "usage"),
        support::json(r#"{"input_tokens":0}"#)
    );
    assert_eq!(support::field(&record, "temperature"), support::json("0"));
    assert_eq!(support::field(&record, "tools"), support::json("[]"));
    assert_eq!(
        support::field(&record, "parallel_tool_calls"),
        Json::Bool(false)
    );
    assert_eq!(support::field(&record, "incomplete_details"), Json::Null);
}

pub(super) fn assert_completed_without_iterating(scenario: &Scenario, warnings: &[String]) {
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "completed");
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("has no body_iterator"))
    );
}
