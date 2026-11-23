use std::sync::atomic::Ordering;

use betterlite_models::decisions::Json;

use super::support::{self, Answer, AnswerStep, REQUEST_BODY, Scenario};

const ITEM_ADDED: &str =
    r#"{"item":{"content":[],"id":"m1","type":"message"},"type":"response.output_item.added"}"#;
const IN_PROGRESS: &str =
    r#"{"response":{"id":"r1","status":"in_progress"},"type":"response.in_progress"}"#;
const DELTA: &str =
    r#"{"content_index":0,"delta":"Hi","item_id":"m1","type":"response.output_text.delta"}"#;
const COMPLETED: &str = r#"{"response":{"id":"r1","model":"b936-test-model","output":[{"content":[{"text":"Hi","type":"output_text"}],"id":"m1","type":"message"}],"usage":{"input_tokens":1,"output_tokens":1}},"type":"response.completed"}"#;

pub(super) async fn disconnected_stream() -> Scenario {
    let mut chunks = support::frames(&[ITEM_ADDED, IN_PROGRESS, DELTA, COMPLETED]);
    chunks.extend(support::frames(&["[DONE]"]));
    let steps: Vec<AnswerStep> = chunks.into_iter().map(AnswerStep::Chunk).collect();
    let mut scenario = support::scenario(&[1000], Answer::Streaming(steps));
    scenario.run().await.expect("the worker completes");
    scenario
}

pub(super) fn assert_provider_call(scenario: &Scenario) {
    let call = scenario.processor.captured();
    assert_eq!(call.route_type, "aresponses");
    assert_eq!(call.model, None);
    assert!(call.skip_pre_call_logic);
    assert!(call.detached_receive_pending);
    assert_eq!(call.data.get("stream"), Some(&Json::Bool(true)));
    assert!(!call.data.contains_key("background"));
    assert_eq!(
        call.data.get("model"),
        Some(&Json::Text(String::from("b936-test-model")))
    );
    assert_eq!(
        call.data.get("input"),
        Some(&Json::Text(String::from("Hello")))
    );
}

pub(super) fn assert_scope_preserved(scenario: &Scenario) {
    let call = scenario.processor.captured();
    assert_eq!(call.scope.method, "POST");
    assert_eq!(call.scope.path, "/v1/responses");
    assert_eq!(
        call.scope.headers,
        vec![
            (
                String::from("authorization"),
                String::from("Bearer sk-b936")
            ),
            (
                String::from("content-type"),
                String::from("application/json")
            ),
        ]
    );
    assert_eq!(call.scope.body, REQUEST_BODY.as_bytes());
}

pub(super) fn assert_single_admission(scenario: &Scenario) {
    assert_eq!(scenario.processor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(scenario.inbound.received.load(Ordering::SeqCst), 0);
}

pub(super) fn assert_record_completed(scenario: &Scenario) {
    let writes = scenario.cache.writes();
    assert_eq!(support::text(&writes[0], "status"), "in_progress");
    assert_eq!(support::text(&writes[1], "status"), "in_progress");
    let record = support::last_write(&scenario.cache);
    assert_eq!(support::text(&record, "status"), "completed");
    assert_eq!(support::first_text(&record), "Hi");
    assert_eq!(
        support::field(&record, "usage"),
        support::json(r#"{"input_tokens":1,"output_tokens":1}"#)
    );
}
