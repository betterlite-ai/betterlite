#![cfg(test)]

use std::num::NonZeroUsize;
use std::time::Duration;

use rstest::rstest;

use super::events::StreamResult;
use super::{SimulatedEvent, StreamPacing, fake_streaming_from_response};
use crate::pydantic_ai_transformation::tests::{Source, response, uuid};

fn stream(result: &str, pacing: StreamPacing) -> Vec<SimulatedEvent> {
    fake_streaming_from_response(
        &response(result),
        "request-1",
        pacing,
        &mut Source::default(),
    )
}

fn json_of(step: &SimulatedEvent) -> String {
    serde_json::to_string(&step.event).unwrap()
}

fn answer(text: &str) -> String {
    format!(
        r#"{{
            "id": "agent-task",
            "contextId": "agent-context",
            "history": [
                {{"role": "user", "messageId": "question-id", "parts": [{{"kind": "text", "text": "question"}}]}},
                {{"role": "agent", "messageId": "answer-id", "parts": [{{"kind": "text", "text": "{text}"}}]}}
            ]
        }}"#
    )
}

fn pacing(chunk_size: usize) -> StreamPacing {
    StreamPacing {
        chunk_size: NonZeroUsize::new(chunk_size).unwrap(),
        delay: Duration::from_millis(10),
    }
}

fn kinds(steps: &[SimulatedEvent]) -> Vec<&'static str> {
    steps
        .iter()
        .map(|step| match &step.event.result {
            StreamResult::Task(result) => result.kind,
            StreamResult::Status(result) => result.kind,
            StreamResult::Artifact(result) => result.kind,
        })
        .collect()
}

fn artifact_texts(steps: &[SimulatedEvent]) -> Vec<String> {
    steps
        .iter()
        .filter_map(|step| match &step.event.result {
            StreamResult::Artifact(result) => result.artifact.parts[0].text.clone(),
            StreamResult::Task(_) | StreamResult::Status(_) => None,
        })
        .collect()
}

fn delays(steps: &[SimulatedEvent]) -> Vec<u128> {
    steps
        .iter()
        .map(|step| step.delay_after.as_millis())
        .collect()
}

#[test]
fn fake_streaming_emits_task_working_artifact_and_completed_in_order() {
    let steps = stream(&answer("hello"), StreamPacing::default());
    assert_eq!(
        kinds(&steps),
        ["task", "status-update", "artifact-update", "status-update"]
    );
}

#[test]
fn fake_streaming_task_event_is_submitted_with_the_input_message() {
    let steps = stream(&answer("hello"), StreamPacing::default());
    assert_eq!(
        json_of(&steps[0]),
        format!(
            r#"{{"id":"request-1","jsonrpc":"2.0","result":{{"contextId":"{context}","history":[{{"contextId":"{context}","kind":"message","messageId":"question-id","parts":[{{"kind":"text","text":"question"}}],"role":"user","taskId":"{task}"}}],"id":"{task}","kind":"task","status":{{"state":"submitted"}}}}}}"#,
            task = uuid(1),
            context = uuid(2),
        )
    );
}

#[rstest]
#[case::no_history("{}", uuid(5), r#"[{"kind":"text","text":""}]"#)]
#[case::only_agent_history(
    r#"{"history": [{"role": "agent", "messageId": "a", "parts": [{"kind": "text", "text": "x"}]}]}"#,
    uuid(4),
    r#"[{"kind":"text","text":""}]"#
)]
#[case::user_without_parts(
    r#"{"history": [{"role": "user", "messageId": "u"}]}"#,
    "u".to_owned(),
    r#"[{"kind":"text","text":""}]"#
)]
#[case::user_with_empty_parts(
    r#"{"history": [{"role": "user", "messageId": "u", "parts": []}]}"#,
    "u".to_owned(),
    "[]"
)]
fn fake_streaming_task_event_defaults_the_input_message(
    #[case] result: &str,
    #[case] message_id: String,
    #[case] parts: &str,
) {
    let steps = stream(result, StreamPacing::default());
    let StreamResult::Task(task) = &steps[0].event.result else {
        panic!("first event is not a task");
    };
    assert_eq!(task.history[0].message_id, message_id);
    assert_eq!(
        serde_json::to_string(&task.history[0].parts).unwrap(),
        parts
    );
}

#[test]
fn fake_streaming_takes_the_first_user_message_as_input() {
    let steps = stream(
        r#"{"history": [
            {"role": "user", "messageId": "first", "parts": [{"kind": "text", "text": "one"}]},
            {"role": "user", "messageId": "second", "parts": [{"kind": "text", "text": "two"}]}
        ]}"#,
        StreamPacing::default(),
    );
    let StreamResult::Task(task) = &steps[0].event.result else {
        panic!("first event is not a task");
    };
    assert_eq!(task.history[0].message_id, "first");
}

#[test]
fn fake_streaming_working_event_is_not_final() {
    let steps = stream(&answer("hello"), StreamPacing::default());
    assert_eq!(
        json_of(&steps[1]),
        format!(
            r#"{{"id":"request-1","jsonrpc":"2.0","result":{{"contextId":"{}","final":false,"kind":"status-update","status":{{"state":"working"}},"taskId":"{}"}}}}"#,
            uuid(2),
            uuid(1)
        )
    );
}

#[test]
fn fake_streaming_completed_event_is_final() {
    let steps = stream(&answer("hello"), StreamPacing::default());
    assert_eq!(
        json_of(&steps[3]),
        format!(
            r#"{{"id":"request-1","jsonrpc":"2.0","result":{{"contextId":"{}","final":true,"kind":"status-update","status":{{"state":"completed"}},"taskId":"{}"}}}}"#,
            uuid(2),
            uuid(1)
        )
    );
}

#[test]
fn fake_streaming_artifact_chunks_share_one_artifact_id() {
    let steps = stream(&answer(&"a".repeat(120)), StreamPacing::default());
    assert_eq!(
        json_of(&steps[2]),
        format!(
            r#"{{"id":"request-1","jsonrpc":"2.0","result":{{"contextId":"{}","kind":"artifact-update","taskId":"{}","artifact":{{"artifactId":"{}","parts":[{{"kind":"text","text":"{}"}}]}}}}}}"#,
            uuid(2),
            uuid(1),
            uuid(3),
            "a".repeat(50)
        )
    );
    let artifact_ids = steps
        .iter()
        .filter_map(|step| match &step.event.result {
            StreamResult::Artifact(result) => Some(result.artifact.artifact_id.clone()),
            StreamResult::Task(_) | StreamResult::Status(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(artifact_ids, [uuid(3), uuid(3), uuid(3)]);
}

#[rstest]
#[case::default_size(120, 50, vec![50, 50, 20])]
#[case::exact_multiple(100, 50, vec![50, 50])]
#[case::shorter_than_a_chunk(7, 50, vec![7])]
#[case::one_over(51, 50, vec![50, 1])]
#[case::custom_size(10, 3, vec![3, 3, 3, 1])]
fn fake_streaming_splits_text_into_chunks(
    #[case] length: usize,
    #[case] chunk_size: usize,
    #[case] lengths: Vec<usize>,
) {
    let steps = stream(&answer(&"x".repeat(length)), pacing(chunk_size));
    let chunk_lengths = artifact_texts(&steps)
        .iter()
        .map(|text| text.chars().count())
        .collect::<Vec<_>>();
    assert_eq!(chunk_lengths, lengths);
}

#[test]
fn fake_streaming_counts_characters_not_utf8_bytes() {
    let steps = stream(&answer(&"\u{e9}".repeat(60)), StreamPacing::default());
    assert_eq!(
        artifact_texts(&steps),
        ["\u{e9}".repeat(50), "\u{e9}".repeat(10)]
    );
}

#[test]
fn fake_streaming_keeps_astral_characters_whole() {
    let steps = stream(&answer("a\u{1f600}b\u{1f600}c"), pacing(2));
    assert_eq!(artifact_texts(&steps), ["a\u{1f600}", "b\u{1f600}", "c"]);
}

#[test]
fn fake_streaming_waits_after_working_and_between_chunks_only() {
    let steps = stream(&answer(&"a".repeat(120)), StreamPacing::default());
    assert_eq!(delays(&steps), [0, 10, 10, 10, 0, 0]);
}

#[test]
fn fake_streaming_empty_text_emits_no_artifact_event() {
    let steps = stream(&answer(""), StreamPacing::default());
    assert_eq!(kinds(&steps), ["task", "status-update", "status-update"]);
    assert_eq!(delays(&steps), [0, 10, 0]);
    assert!(json_of(&steps[2]).contains(r#""final":true"#));
}

#[test]
fn fake_streaming_generates_fresh_ids_instead_of_reusing_the_response() {
    let steps = stream(&answer("hello"), StreamPacing::default());
    let text = steps.iter().map(json_of).collect::<String>();
    assert!(text.contains(&uuid(1)));
    assert!(!text.contains("agent-task"));
    assert!(!text.contains("agent-context"));
}

#[test]
fn fake_streaming_stamps_the_request_id_on_every_event() {
    let steps = stream(&answer(&"a".repeat(60)), StreamPacing::default());
    assert_eq!(steps.len(), 5);
    assert!(
        steps
            .iter()
            .all(|step| step.event.id == "request-1" && step.event.jsonrpc == "2.0")
    );
}
