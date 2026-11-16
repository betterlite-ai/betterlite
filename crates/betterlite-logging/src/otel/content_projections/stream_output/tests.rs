#![cfg(test)]

use std::sync::Mutex;

use rstest::rstest;

use super::{RelayedChunk, StreamOutputAssembler, stream_output};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

#[derive(Debug, PartialEq, Eq)]
enum Call {
    Sse(Vec<Vec<u8>>),
    Chat {
        chunks: Vec<String>,
        messages: Option<Vec<String>>,
    },
}

struct FakeAssembler {
    assembled: Option<OtelModelValue>,
    calls: Mutex<Vec<Call>>,
}

impl FakeAssembler {
    fn returning(text: Option<&str>) -> Self {
        Self {
            assembled: text.map(value),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn recorded(&self) -> Vec<Call> {
        std::mem::take(&mut *self.calls.lock().expect("calls"))
    }
}

fn json(item: &OtelModelValue) -> String {
    serde_json::to_string(item).expect("json")
}

impl StreamOutputAssembler for FakeAssembler {
    fn assemble_sse(&self, frames: &[Vec<u8>]) -> Option<OtelModelValue> {
        self.calls
            .lock()
            .expect("calls")
            .push(Call::Sse(frames.to_vec()));
        self.assembled.clone()
    }

    fn assemble_chat(
        &self,
        chunks: &[OtelModelValue],
        messages: Option<&[OtelModelValue]>,
    ) -> Option<OtelModelValue> {
        self.calls.lock().expect("calls").push(Call::Chat {
            chunks: chunks.iter().map(json).collect(),
            messages: messages.map(|items| items.iter().map(json).collect()),
        });
        self.assembled.clone()
    }
}

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

fn data(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("request data")
}

fn raw(text: &str) -> RelayedChunk {
    RelayedChunk::Raw(text.as_bytes().to_vec())
}

fn chat(text: &str) -> RelayedChunk {
    RelayedChunk::Chat(value(text))
}

fn completed(text: &str) -> RelayedChunk {
    RelayedChunk::Completed(value(text))
}

const REPLY: &str = r#"{"choices": [{"message": {"role": "assistant", "content": "hi"}}]}"#;
const REPLY_OUTPUT: &str = r#"[{"content":"hi","role":"assistant"}]"#;

#[test]
fn no_chunks_give_none_without_asking_the_assembler() {
    let assembler = FakeAssembler::returning(Some(REPLY));
    assert_eq!(stream_output(&[], &data("{}"), &assembler), None);
    assert!(assembler.recorded().is_empty());
}

#[test]
fn one_raw_frame_makes_the_stream_raw_and_only_the_raw_frames_are_assembled_in_order() {
    let assembler = FakeAssembler::returning(Some(REPLY));
    let chunks = [
        chat(r#"{"id": "typed"}"#),
        raw("event: message_start\n"),
        completed(r#"{"output": []}"#),
        raw("data: {}\n\n"),
    ];
    let output = stream_output(&chunks, &data(r#"{"messages": []}"#), &assembler);
    assert_eq!(output.as_deref(), Some(REPLY_OUTPUT));
    assert_eq!(
        assembler.recorded(),
        [Call::Sse(vec![
            b"event: message_start\n".to_vec(),
            b"data: {}\n\n".to_vec()
        ])]
    );
}

#[test]
fn a_raw_stream_never_reaches_the_chat_assembler() {
    let assembler = FakeAssembler::returning(None);
    let chunks = [raw("a"), chat(r#"{"id": "1"}"#)];
    assert_eq!(stream_output(&chunks, &data("{}"), &assembler), None);
    assert!(
        assembler
            .recorded()
            .iter()
            .all(|call| matches!(call, Call::Sse(_)))
    );
}

#[test]
fn a_stream_of_chat_chunks_goes_to_the_chat_assembler_with_the_request_messages() {
    let assembler = FakeAssembler::returning(Some(REPLY));
    let chunks = [chat(r#"{"id": "1"}"#), chat(r#"{"id": "2"}"#)];
    let request = data(r#"{"messages": [{"role": "user", "content": "q"}]}"#);
    assert_eq!(
        stream_output(&chunks, &request, &assembler).as_deref(),
        Some(REPLY_OUTPUT)
    );
    assert_eq!(
        assembler.recorded(),
        [Call::Chat {
            chunks: vec![r#"{"id":"1"}"#.to_owned(), r#"{"id":"2"}"#.to_owned()],
            messages: Some(vec![r#"{"content":"q","role":"user"}"#.to_owned()]),
        }]
    );
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"messages": null}"#)]
fn absent_and_null_messages_both_reach_the_chat_assembler_as_none(#[case] request: &str) {
    let assembler = FakeAssembler::returning(Some(REPLY));
    let chunks = [chat(r#"{"id": "1"}"#)];
    assert!(stream_output(&chunks, &data(request), &assembler).is_some());
    assert_eq!(
        assembler.recorded(),
        [Call::Chat {
            chunks: vec![r#"{"id":"1"}"#.to_owned()],
            messages: None,
        }]
    );
}

#[test]
fn an_empty_messages_list_is_passed_on_as_an_empty_list() {
    let assembler = FakeAssembler::returning(Some(REPLY));
    let chunks = [chat(r#"{"id": "1"}"#)];
    assert!(stream_output(&chunks, &data(r#"{"messages": []}"#), &assembler).is_some());
    assert_eq!(
        assembler.recorded(),
        [Call::Chat {
            chunks: vec![r#"{"id":"1"}"#.to_owned()],
            messages: Some(Vec::new()),
        }]
    );
}

#[rstest]
#[case::text(r#"{"messages": "q"}"#)]
#[case::object(r#"{"messages": {"a": 1}}"#)]
fn messages_that_are_not_a_list_give_none_before_any_assembly(#[case] request: &str) {
    let assembler = FakeAssembler::returning(Some(REPLY));
    let chunks = [chat(r#"{"id": "1"}"#)];
    assert_eq!(stream_output(&chunks, &data(request), &assembler), None);
    assert!(assembler.recorded().is_empty());
}

#[test]
fn the_last_completed_response_wins() {
    let assembler = FakeAssembler::returning(None);
    let chunks = [
        completed(r#"{"output": [{"type": "message", "id": "first"}]}"#),
        RelayedChunk::Other(value(r#"{"type": "response.output_text.delta"}"#)),
        completed(r#"{"output": [{"type": "message", "id": "second"}]}"#),
        RelayedChunk::Other(value(r#"{"type": "ping"}"#)),
    ];
    let output = stream_output(&chunks, &data("{}"), &assembler);
    assert_eq!(
        output.as_deref(),
        Some(r#"[{"id":"second","type":"message"}]"#)
    );
    assert!(assembler.recorded().is_empty());
}

#[test]
fn a_completed_response_without_output_projects_to_null_text() {
    let assembler = FakeAssembler::returning(None);
    let output = stream_output(&[completed(r#"{"id": "r"}"#)], &data("{}"), &assembler);
    assert_eq!(output.as_deref(), Some("null"));
}

#[rstest]
#[case::only_other_chunks(vec![RelayedChunk::Other(value("{}"))])]
#[case::chat_mixed_with_other(vec![chat(r#"{"id": "1"}"#), RelayedChunk::Other(value("{}"))])]
fn a_stream_without_a_completed_event_gives_none(#[case] chunks: Vec<RelayedChunk>) {
    let assembler = FakeAssembler::returning(Some(REPLY));
    assert_eq!(stream_output(&chunks, &data("{}"), &assembler), None);
    assert!(assembler.recorded().is_empty());
}

#[test]
fn a_chat_chunk_beside_a_completed_one_is_not_a_chat_stream() {
    let assembler = FakeAssembler::returning(Some(REPLY));
    let chunks = [chat(r#"{"id": "1"}"#), completed(r#"{"output": []}"#)];
    assert_eq!(
        stream_output(&chunks, &data("{}"), &assembler).as_deref(),
        Some("[]")
    );
    assert!(assembler.recorded().is_empty());
}

#[rstest]
#[case::raw(vec![raw("a")])]
#[case::chat(vec![chat(r#"{"id": "1"}"#)])]
fn a_port_that_fails_gives_none(#[case] chunks: Vec<RelayedChunk>) {
    let assembler = FakeAssembler::returning(None);
    assert_eq!(stream_output(&chunks, &data("{}"), &assembler), None);
}

#[test]
fn a_port_result_is_projected_as_a_model_response_never_as_an_anthropic_message() {
    let assembler = FakeAssembler::returning(Some(
        r#"{"type": "message", "role": "assistant", "content": "x"}"#,
    ));
    let output = stream_output(&[raw("a")], &data("{}"), &assembler);
    assert_eq!(output.as_deref(), Some(r#"[{"role":"assistant"}]"#));
}
