#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};

use betterlite_core::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use rstest::rstest;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use super::events::StreamResult;
use super::streaming::{EventContent, stream_event};
use super::wire::{A2AMessage, CompletionParams, MessageSendParams, ModelResponse};
use super::{
    A2AStreamingContext, EventSource, apply_forward_metadata, extract_text, forward_metadata,
    response_to_a2a, to_openai_messages,
};

pub(super) struct Source {
    ids: VecDeque<Uuid>,
    timestamp: DateTime<Utc>,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            ids: (1..=20).map(Uuid::from_u128).collect(),
            timestamp: "2026-01-02T03:04:05.123456Z".parse().unwrap(),
        }
    }
}

impl EventSource for Source {
    fn next_uuid(&mut self) -> Uuid {
        self.ids.pop_front().unwrap()
    }
    fn now(&mut self) -> DateTime<Utc> {
        self.timestamp
    }
}

#[rstest]
#[case::missing_kind(r#"[{"text":"a"},{"text":"b"}]"#, "a\nb")]
#[case::empty_text(r#"[{"text":""},{"text":"b"}]"#, "\nb")]
#[case::null_kind(r#"[{"kind":null,"text":"a"}]"#, "a")]
#[case::empty_kind(r#"[{"kind":"","text":"a"}]"#, "a")]
#[case::malformed(r#"[null,1,true,[],"bad",{}, {"text":null},{"text":2},{"kind":"image","text":"skip"},{"kind":"text","text":"yes"}]"#, "yes")]
#[case::empty("[]", "")]
fn extracts_parts(#[case] input: &str, #[case] expected: &str) {
    let parts: Vec<AgentJson> = serde_json::from_str(input).unwrap();
    assert_eq!(extract_text(&parts), expected);
}

#[rstest]
#[case::user("user")]
#[case::assistant("assistant")]
#[case::system("system")]
#[case::developer("developer")]
#[case::agent("agent")]
#[case::empty("")]
fn passes_roles_without_metadata(#[case] role: &str) {
    let message: A2AMessage = serde_json::from_value(
        serde_json::from_str::<AgentJson>(&format!(
            r#"{{"role":{},"parts":[{{"text":"hello"}}],"metadata":{{"a":1}}}}"#,
            serde_json::to_string(&role).unwrap()
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(to_openai_messages(&message)).unwrap(),
        serde_json::from_str::<AgentJson>(&format!(
            r#"[{{"role":{},"content":"hello"}}]"#,
            serde_json::to_string(&role).unwrap()
        ))
        .unwrap()
    );
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"role":null,"parts":null,"messageId":null}"#)]
fn null_fields_take_defaults(#[case] input: &str) {
    let message: A2AMessage = serde_json::from_str(input).unwrap();
    assert_eq!(
        serde_json::to_value(to_openai_messages(&message)).unwrap(),
        serde_json::from_str::<AgentJson>(r#"[{"role":"user","content":""}]"#).unwrap()
    );
    let mut source = Source::default();
    let context = A2AStreamingContext::new("req".to_owned(), message, &mut source);
    let event = stream_event(&context, EventContent::Task, &mut source);
    let StreamResult::Task(task) = event.result else {
        panic!("expected task");
    };
    assert_eq!(
        task.history[0].message_id,
        Uuid::from_u128(3).simple().to_string()
    );
    assert_eq!(task.history[0].role, "user");
    assert!(task.history[0].parts.is_empty());
}

#[rstest]
#[case::role(r#"{"role":1}"#, "role")]
#[case::parts(r#"{"parts":{}}"#, "parts")]
#[case::id(r#"{"messageId":false}"#, "messageId")]
fn rejects_wrong_field_types(#[case] input: &str, #[case] field: &str) {
    let parsed = serde_json::from_str::<A2AMessage>(input);
    assert!(parsed.is_err(), "{field} must be typed");
}

#[test]
fn merges_metadata_with_owner_precedence() {
    let message = serde_json::from_value(
        serde_json::from_str::<AgentJson>(r#"{"metadata":{"message":true,"same":"message"}}"#)
            .unwrap(),
    )
    .unwrap();
    let params = serde_json::from_value(
        serde_json::from_str::<AgentJson>(
            r#"{"message":{},"metadata":{"params":true,"same":"params"}}"#,
        )
        .unwrap(),
    )
    .unwrap();
    let mut completion = serde_json::from_value(serde_json::from_str::<AgentJson>(r#"{"metadata":{"spend":true},"model":"agent","extra_body":{"custom":1,"metadata":{"same":"configured","owner":2}}}"#).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(forward_metadata(&message, Some(&params))).unwrap(),
        serde_json::from_str::<AgentJson>(r#"{"message":true,"params":true,"same":"message"}"#)
            .unwrap()
    );
    apply_forward_metadata(&mut completion, &message, Some(&params));
    assert_eq!(
        serde_json::to_value(completion).unwrap(),
        serde_json::from_str::<AgentJson>(r#"{"metadata":{"spend":true},"model":"agent","extra_body":{"custom":1,"metadata":{"same":"configured","owner":2,"message":true,"params":true}}}"#).unwrap()
    );
}

#[rstest]
#[case::absent("{}")]
#[case::empty(r#"{"metadata":{}}"#)]
#[case::null(r#"{"metadata":null}"#)]
#[case::non_object(r#"{"metadata":[1]}"#)]
fn empty_metadata_leaves_completion(#[case] input: &str) {
    let message: A2AMessage = serde_json::from_str(input).unwrap();
    let params = MessageSendParams {
        message: message.clone(),
        metadata: message.metadata.clone(),
        extra: BTreeMap::new(),
    };
    assert!(forward_metadata(&message, Some(&params)).is_none());
    let mut completion: CompletionParams = serde_json::from_value(
        serde_json::from_str::<AgentJson>(r#"{"extra_body":"untouched"}"#).unwrap(),
    )
    .unwrap();
    let before = completion.clone();
    apply_forward_metadata(&mut completion, &message, Some(&params));
    assert_eq!(completion, before);
}

#[rstest]
#[case::absent(serde_json::from_str::<AgentJson>("{}").unwrap())]
#[case::null(serde_json::from_str::<AgentJson>(r#"{"extra_body":null}"#).unwrap())]
#[case::non_object(serde_json::from_str::<AgentJson>(r#"{"extra_body":5}"#).unwrap())]
#[case::invalid_metadata(serde_json::from_str::<AgentJson>(r#"{"extra_body":{"metadata":[]}}"#).unwrap())]
fn replaces_invalid_metadata(#[case] input: AgentJson) {
    let mut completion = serde_json::from_value(input).unwrap();
    let message = serde_json::from_value(
        serde_json::from_str::<AgentJson>(r#"{"metadata":{"a":1}}"#).unwrap(),
    )
    .unwrap();
    apply_forward_metadata(&mut completion, &message, None);
    assert_eq!(
        serde_json::to_value(completion).unwrap(),
        serde_json::from_str::<AgentJson>(r#"{"extra_body":{"metadata":{"a":1}}}"#).unwrap()
    );
}

#[rstest]
#[case::first(
    r#"{"choices":[{"message":{"content":"answer"}},{"message":{"content":"ignore"}}]}"#,
    "answer"
)]
#[case::empty(r#"{"choices":[]}"#, "")]
#[case::missing_message(r#"{"choices":[{}]}"#, "")]
#[case::null_message(r#"{"choices":[{"message":null}]}"#, "")]
#[case::null_content(r#"{"choices":[{"message":{"content":null}}]}"#, "")]
fn converts_first_response(#[case] input: &str, #[case] text: &str) {
    let response: ModelResponse = serde_json::from_str(input).unwrap();
    let mut source = Source::default();
    let converted = response_to_a2a(Some(&response), Some("req"), &mut source);
    assert_eq!(
        serde_json::to_value(converted).unwrap(),
        serde_json::from_str::<AgentJson>(&format!(r#"{{"jsonrpc":"2.0","id":"req","result":{{"kind":"message","role":"agent","parts":[{{"kind":"text","text":{}}}],"messageId":"00000000000000000000000000000001"}}}}"#, serde_json::to_string(&text).unwrap())).unwrap()
    );
}

#[test]
fn non_model_response_is_empty() {
    let mut source = Source::default();
    let first = response_to_a2a(None, None, &mut source);
    let second = response_to_a2a(None, None, &mut source);
    assert_eq!(first.result.parts[0].text, "");
    assert!(first.id.is_none());
    assert_ne!(first.result.message_id, second.result.message_id);
}

#[test]
fn initializes_context_and_task_history() {
    let message: A2AMessage = serde_json::from_value(serde_json::from_str::<AgentJson>(r#"{"role":"developer","messageId":"original","parts":[{"kind":"data","data":{"a":1}}, {"text":"hello"}]}"#).unwrap()).unwrap();
    let mut source = Source::default();
    let context = A2AStreamingContext::new("req".to_owned(), message.clone(), &mut source);
    assert_eq!(context.input_message, message);
    assert!(context.accumulated_text.is_empty());
    assert!(!context.has_emitted_task);
    assert!(!context.has_emitted_working);
    assert_eq!(context.task_id, Uuid::from_u128(1).to_string());
    assert_eq!(context.context_id, Uuid::from_u128(2).to_string());
    let event = stream_event(&context, EventContent::Task, &mut source);
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        serde_json::from_str::<AgentJson>(r#"{"jsonrpc":"2.0","id":"req","result":{"contextId":"00000000-0000-0000-0000-000000000002","id":"00000000-0000-0000-0000-000000000001","kind":"task","status":{"state":"submitted"},"history":[{"kind":"message","messageId":"original","role":"developer","parts":[{"kind":"data","data":{"a":1}},{"text":"hello"}],"taskId":"00000000-0000-0000-0000-000000000001","contextId":"00000000-0000-0000-0000-000000000002"}]}}"#).unwrap()
    );
}

#[rstest]
#[case::working("working", false, Some("thinking"), true)]
#[case::empty("working", false, Some(""), false)]
#[case::no_message("working", false, None, false)]
#[case::completed("completed", true, Some("ignored"), false)]
#[case::other("failed", true, Some("ignored"), false)]
fn status_shape(
    #[case] state: &str,
    #[case] final_event: bool,
    #[case] text: Option<&str>,
    #[case] has_message: bool,
) {
    let mut source = Source::default();
    let message = serde_json::from_str("{}").unwrap();
    let context = A2AStreamingContext::new("req".to_owned(), message, &mut source);
    let event = stream_event(
        &context,
        EventContent::Status {
            state,
            final_event,
            message: text,
        },
        &mut source,
    );
    let mut status = serde_json::from_str::<AgentJson>(&format!(
        r#"{{"state":{},"timestamp":"2026-01-02T03:04:05.123456+00:00"}}"#,
        serde_json::to_string(&state).unwrap()
    ))
    .unwrap();
    if has_message {
        status["message"] = serde_json::from_str::<AgentJson>(&format!(r#"{{"contextId":"00000000-0000-0000-0000-000000000002","kind":"message","messageId":"00000000-0000-0000-0000-000000000003","parts":[{{"kind":"text","text":{}}}],"role":"agent","taskId":"00000000-0000-0000-0000-000000000001"}}"#, serde_json::to_string(&text).unwrap())).unwrap();
    }
    assert_eq!(
        serde_json::to_value(event).unwrap(),
        serde_json::from_str::<AgentJson>(&format!(r#"{{"id":"req","jsonrpc":"2.0","result":{{"contextId":"00000000-0000-0000-0000-000000000002","final":{},"kind":"status-update","status":{},"taskId":"00000000-0000-0000-0000-000000000001"}}}}"#, serde_json::to_string(&final_event).unwrap(), serde_json::to_string(&status).unwrap())).unwrap()
    );
}

#[test]
fn timestamp_at_whole_second() {
    let mut source = Source {
        ids: (1..=3).map(Uuid::from_u128).collect(),
        timestamp: "2026-01-02T03:04:05Z".parse().unwrap(),
    };
    let context = A2AStreamingContext::new(
        "req".to_owned(),
        serde_json::from_str("{}").unwrap(),
        &mut source,
    );
    let event = stream_event(
        &context,
        EventContent::Status {
            state: "completed",
            final_event: true,
            message: None,
        },
        &mut source,
    );
    let value = serde_json::to_value(event).unwrap();
    assert_eq!(
        value["result"]["status"]["timestamp"],
        "2026-01-02T03:04:05+00:00"
    );
}

#[test]
fn artifact_shape_and_fresh_ids() {
    let mut source = Source::default();
    let context = A2AStreamingContext::new(
        "req".to_owned(),
        serde_json::from_str("{}").unwrap(),
        &mut source,
    );
    let first = stream_event(&context, EventContent::Artifact("answer"), &mut source);
    let second = stream_event(&context, EventContent::Artifact(""), &mut source);
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::from_str::<AgentJson>(r#"{"id":"req","jsonrpc":"2.0","result":{"artifact":{"artifactId":"00000000-0000-0000-0000-000000000003","name":"response","parts":[{"kind":"text","text":"answer"}]},"contextId":"00000000-0000-0000-0000-000000000002","kind":"artifact-update","taskId":"00000000-0000-0000-0000-000000000001"}}"#).unwrap()
    );
    assert_ne!(
        serde_json::to_value(first).unwrap()["result"]["artifact"]["artifactId"],
        serde_json::to_value(second).unwrap()["result"]["artifact"]["artifactId"]
    );
    let working = stream_event(
        &context,
        EventContent::Status {
            state: "working",
            final_event: false,
            message: Some("one"),
        },
        &mut source,
    );
    let again = stream_event(
        &context,
        EventContent::Status {
            state: "working",
            final_event: false,
            message: Some("two"),
        },
        &mut source,
    );
    assert_ne!(
        serde_json::to_value(working).unwrap()["result"]["status"]["message"]["messageId"],
        serde_json::to_value(again).unwrap()["result"]["status"]["message"]["messageId"]
    );
}

#[rstest]
fn replays_completion_corpus(#[files("fuzz/corpus/completion_bridge/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(message) = serde_json::from_slice::<A2AMessage>(&bytes) {
        drop(to_openai_messages(&message));
        let mut completion = CompletionParams::default();
        apply_forward_metadata(&mut completion, &message, None);
    }
    drop(serde_json::from_slice::<CompletionParams>(&bytes));
    drop(serde_json::from_slice::<MessageSendParams>(&bytes));
    drop(serde_json::from_slice::<ModelResponse>(&bytes));
}
