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
const COMPLETED_WITH_FIELDS: &str = r#"{"response":{"incomplete_details":{},"instructions":"","max_output_tokens":0,"model":"b936-model","parallel_tool_calls":false,"previous_response_id":"resp_prev","reasoning":{"effort":"low"},"status":"completed","store":false,"temperature":0,"text":{"format":{"type":"text"}},"tool_choice":"auto","tools":[],"top_p":0.5,"truncation":"disabled","usage":{"input_tokens":7,"output_tokens":0,"total_tokens":7},"user":""},"type":"response.completed"}"#;
// every ResponsesAPIResponse field background_streaming.py copies into the terminal update
const TERMINAL_NAMES: [&str; 16] = [
    "usage",
    "reasoning",
    "tool_choice",
    "tools",
    "model",
    "instructions",
    "temperature",
    "top_p",
    "max_output_tokens",
    "previous_response_id",
    "text",
    "truncation",
    "parallel_tool_calls",
    "user",
    "store",
    "incomplete_details",
];

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
    with_events(&[ITEM_ADDED, COMPLETED_WITH_FIELDS]).await
}

pub(super) async fn absent_optional_keys() -> Scenario {
    with_events(&[
        "{}",
        r#"{"type":"response.output_item.added"}"#,
        r#"{"type":"response.content_part.added"}"#,
        r#"{"type":"response.content_part.done"}"#,
        r#"{"type":"response.output_text.delta"}"#,
        r#"{"type":"response.completed"}"#,
    ])
    .await
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
    assert_eq!(support::text(&record, "status"), "completed");
    let event = support::json(COMPLETED_WITH_FIELDS);
    let Json::Object(response) = support::field(&event, "response") else {
        panic!("the completed event carries a response object");
    };
    for name in TERMINAL_NAMES {
        assert_eq!(
            support::field(&record, name),
            response[name],
            "terminal field {name}"
        );
    }
    assert_eq!(support::field(&record, "error"), Json::Null);
}

pub(super) fn assert_defaults_completed(scenario: &Scenario) {
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "completed");
    assert_eq!(support::field(&record, "output"), Json::List(Vec::new()));
    assert_eq!(support::field(&record, "error"), Json::Null);
    assert_eq!(support::field(&record, "usage"), Json::Null);
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
