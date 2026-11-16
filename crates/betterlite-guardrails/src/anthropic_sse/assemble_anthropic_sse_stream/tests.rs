#![cfg(test)]

use std::future::Future;
use std::sync::Mutex;

use betterlite_core::api::messages_chat_output::records::ResponseRecord;

use super::{SseAssembler, assemble_anthropic_sse_stream};
use crate::anthropic_sse::tests::support::{
    MESSAGE_START, PING, bytes_chunk, object_chunk, record, text_chunk,
};

const UPSTREAM: &str = r#"{"id": "chatcmpl-up", "model": "up-model"}"#;

struct FakeAssembler {
    reply: Option<ResponseRecord>,
    calls: Mutex<Vec<(String, String)>>,
}

impl FakeAssembler {
    fn answering(reply: Option<ResponseRecord>) -> Self {
        Self {
            reply,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }
}

impl SseAssembler for FakeAssembler {
    fn assemble(
        &self,
        sse_stream: &str,
        model: &str,
    ) -> impl Future<Output = Option<ResponseRecord>> + Send {
        self.calls
            .lock()
            .unwrap()
            .push((sse_stream.to_owned(), model.to_owned()));
        let reply = self.reply.clone();
        async move { reply }
    }
}

#[tokio::test]
async fn assemble_anthropic_sse_stream_keeps_the_wire_shape_by_default() {
    let assembler = FakeAssembler::answering(Some(record(UPSTREAM)));
    let chunks = [text_chunk(PING), bytes_chunk(MESSAGE_START.as_bytes())];
    let restored = assemble_anthropic_sse_stream(&chunks, &assembler, false)
        .await
        .unwrap();
    assert_eq!(restored.id, "chatcmpl-up");
    assert_eq!(restored.model, "up-model");
    let joined = format!("{PING}{MESSAGE_START}");
    assert_eq!(assembler.calls(), [(joined, String::new())]);
}

#[tokio::test]
async fn assemble_anthropic_sse_stream_restores_the_message_id_and_model() {
    let assembler = FakeAssembler::answering(Some(record(UPSTREAM)));
    let chunks = [text_chunk(MESSAGE_START)];
    let restored = assemble_anthropic_sse_stream(&chunks, &assembler, true)
        .await
        .unwrap();
    assert_eq!(restored.id, "msg_1");
    assert_eq!(restored.model, "claude-x");
    assert_eq!(
        assembler.calls(),
        [(MESSAGE_START.to_owned(), "claude-x".to_owned())]
    );
}

#[tokio::test]
async fn assemble_anthropic_sse_stream_restores_only_what_message_start_carries() {
    let start = "data: {\"type\": \"message_start\", \"message\": {\"id\": \"msg_2\", \"model\": \"\"}}\n\n";
    let assembler = FakeAssembler::answering(Some(record(UPSTREAM)));
    let restored = assemble_anthropic_sse_stream(&[text_chunk(start)], &assembler, true)
        .await
        .unwrap();
    assert_eq!(restored.id, "msg_2");
    assert_eq!(restored.model, "up-model");
    let bare = "data: {\"type\": \"message_start\", \"message\": {}}\n\n";
    let unchanged = assemble_anthropic_sse_stream(&[text_chunk(bare)], &assembler, true)
        .await
        .unwrap();
    assert_eq!(unchanged.id, "chatcmpl-up");
    assert_eq!(unchanged.model, "up-model");
}

#[tokio::test]
async fn assemble_anthropic_sse_stream_is_none_without_a_message_start() {
    let assembler = FakeAssembler::answering(Some(record(UPSTREAM)));
    let chunks = [text_chunk(PING)];
    assert!(
        assemble_anthropic_sse_stream(&chunks, &assembler, false)
            .await
            .is_none()
    );
    assert!(assembler.calls().is_empty());
}

#[tokio::test]
async fn assemble_anthropic_sse_stream_is_none_when_message_start_has_no_message_object() {
    let start = "data: {\"type\": \"message_start\", \"message\": \"msg_1\"}\n\n";
    let assembler = FakeAssembler::answering(Some(record(UPSTREAM)));
    assert!(
        assemble_anthropic_sse_stream(&[text_chunk(start)], &assembler, false)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn assemble_anthropic_sse_stream_is_none_for_bytes_that_are_not_utf8() {
    let mut frame = MESSAGE_START.as_bytes().to_vec();
    frame.push(0xFF);
    let assembler = FakeAssembler::answering(Some(record(UPSTREAM)));
    assert!(
        assemble_anthropic_sse_stream(&[bytes_chunk(&frame)], &assembler, false)
            .await
            .is_none()
    );
    assert!(assembler.calls().is_empty());
}

#[tokio::test]
async fn assemble_anthropic_sse_stream_is_none_when_the_stream_does_not_assemble() {
    let assembler = FakeAssembler::answering(None);
    let chunks = [text_chunk(MESSAGE_START), object_chunk()];
    assert!(
        assemble_anthropic_sse_stream(&chunks, &assembler, true)
            .await
            .is_none()
    );
}
