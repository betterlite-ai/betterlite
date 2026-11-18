#![cfg(test)]

use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use aws_smithy_types::str_bytes::StrBytes;
use betterlite_http::error::HttpError;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};

use crate::sagemaker_stream::error::SagemakerStreamError;
use crate::sagemaker_stream::events::{EventItem, EventState};
use crate::sagemaker_stream::{ErrorPolicy, SagemakerChunk, SagemakerEvents};

const MODEL: &str = "sagemaker-model";

fn text(value: &str) -> HeaderValue {
    HeaderValue::String(StrBytes::from(value.to_owned()))
}

fn frame(headers: &[(&str, HeaderValue)], payload: &[u8]) -> Vec<u8> {
    let headers = headers
        .iter()
        .map(|(name, value)| Header::new((*name).to_owned(), value.clone()))
        .collect();
    let message = Message::new_from_parts(headers, payload.to_vec());
    let mut out = Vec::new();
    write_message_to(&message, &mut out).unwrap();
    out
}

fn event(payload: &[u8]) -> Vec<u8> {
    let headers = [
        (":message-type", text("event")),
        (":event-type", text("PayloadPart")),
    ];
    frame(&headers, payload)
}

fn exception(payload: &[u8]) -> Vec<u8> {
    let headers = [
        (":message-type", text("exception")),
        (":exception-type", text("ModelStreamError")),
    ];
    frame(&headers, payload)
}

fn corrupted(frame: &[u8]) -> Vec<u8> {
    let mut bytes = frame.to_vec();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    bytes
}

fn source(chunks: Vec<Vec<u8>>) -> Vec<Result<Vec<u8>, HttpError>> {
    let mut items = Vec::new();
    for chunk in chunks {
        items.push(Ok(chunk));
    }
    items
}

fn stream<Chunks>(chunks: Chunks, messages: bool, policy: ErrorPolicy) -> SagemakerEvents<Chunks> {
    SagemakerEvents {
        state: EventState {
            model: MODEL.to_owned(),
            is_messages_api: messages,
            policy,
            frames: bytes::BytesMut::new(),
            accumulated: String::new(),
        },
        chunks,
        pending: VecDeque::new(),
        done: false,
    }
}

fn sync_items(chunks: Vec<Vec<u8>>, messages: bool, policy: ErrorPolicy) -> Vec<EventItem> {
    stream(source(chunks).into_iter(), messages, policy).collect()
}

fn async_items(chunks: Vec<Vec<u8>>, messages: bool, policy: ErrorPolicy) -> Vec<EventItem> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let mut events = stream(futures_util::stream::iter(source(chunks)), messages, policy);
        let mut items = Vec::new();
        while let Some(item) = futures_util::StreamExt::next(&mut events).await {
            items.push(item);
        }
        items
    })
}

fn token(item: &EventItem) -> &GenericChunk {
    let Ok(Some(SagemakerChunk::Token(chunk))) = item else {
        panic!("expected a token chunk, got {item:?}");
    };
    chunk
}

fn chat(item: &EventItem) -> &Chunk {
    let Ok(Some(SagemakerChunk::Chat(chunk))) = item else {
        panic!("expected a chat chunk, got {item:?}");
    };
    chunk
}

#[derive(Clone, Default)]
struct Lines {
    entries: Arc<Mutex<Vec<(Level, String)>>>,
}

impl Lines {
    fn entries(&self) -> Vec<(Level, String)> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn text(&self) -> String {
        let mut text = String::new();
        for (level, message) in self.entries() {
            text.push_str(&level.to_string());
            text.push(' ');
            text.push_str(&message);
            text.push('\n');
        }
        text
    }
}

struct MessageVisitor<'text> {
    message: &'text mut String,
}

impl Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            *self.message = format!("{value:?}").trim_matches('"').to_owned();
        }
    }
}

impl<S: Subscriber> Layer<S> for Lines {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, S>) {
        let mut message = String::new();
        event.record(&mut MessageVisitor {
            message: &mut message,
        });
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), message));
    }
}

fn capture() -> (Lines, tracing::subscriber::DefaultGuard) {
    let lines = Lines::default();
    let guard = set_default(tracing_subscriber::registry().with(lines.clone()));
    (lines, guard)
}

#[test]
fn a_whole_object_in_one_event_yields_one_token_chunk() {
    let items = sync_items(
        vec![event(br#"{"token": {"text": "hello"}, "index": 1}"#)],
        false,
        ErrorPolicy::Propagate,
    );

    assert_eq!(items.len(), 1);
    let chunk = token(&items[0]);
    assert_eq!(chunk.text, "hello");
    assert_eq!(chunk.index, Some(1));
    assert!(!chunk.is_finished);
    assert_eq!(chunk.finish_reason, "");
    assert!(chunk.usage.is_none());
    assert!(chunk.tool_use.is_none());
}

#[test]
fn a_missing_token_and_index_read_as_an_empty_zero_chunk() {
    let items = sync_items(vec![event(b"{}")], false, ErrorPolicy::Propagate);

    let chunk = token(&items[0]);
    assert_eq!(chunk.text, "");
    assert_eq!(chunk.index, Some(0));
    assert!(!chunk.is_finished);
}

#[test]
fn a_null_token_and_index_read_as_an_empty_zero_chunk() {
    let items = sync_items(
        vec![event(br#"{"token": null, "index": null}"#)],
        false,
        ErrorPolicy::Propagate,
    );

    let chunk = token(&items[0]);
    assert_eq!(chunk.text, "");
    assert_eq!(chunk.index, Some(0));
}

#[test]
fn the_end_of_text_token_finishes_the_stream() {
    let items = sync_items(
        vec![event(
            br#"{"token": {"text": "<|endoftext|>"}, "index": 4}"#,
        )],
        false,
        ErrorPolicy::Propagate,
    );

    let chunk = token(&items[0]);
    assert_eq!(chunk.text, "");
    assert_eq!(chunk.index, Some(4));
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, "stop");
}

#[test]
fn a_frame_split_across_chunks_is_joined() {
    let bytes = event(br#"{"token": {"text": "split"}}"#);
    let chunks = vec![
        bytes[..5].to_vec(),
        bytes[5..17].to_vec(),
        bytes[17..].to_vec(),
    ];

    let items = sync_items(chunks, false, ErrorPolicy::Propagate);

    assert_eq!(items.len(), 1);
    assert_eq!(token(&items[0]).text, "split");
}

#[test]
fn a_json_object_split_across_events_is_joined() {
    let items = sync_items(
        vec![event(br#"{"token": {"te"#), event(br#"xt": "joined"}}"#)],
        false,
        ErrorPolicy::Propagate,
    );

    assert_eq!(items.len(), 1);
    assert_eq!(token(&items[0]).text, "joined");
}

#[test]
fn the_data_prefix_and_blank_lines_are_removed() {
    let items = sync_items(
        vec![event(b"data: {\"token\":\n\n{\"text\": \"prefixed\"}}")],
        false,
        ErrorPolicy::Propagate,
    );

    assert_eq!(token(&items[0]).text, "prefixed");
}

#[test]
fn a_data_prefix_without_a_space_is_removed() {
    let items = sync_items(
        vec![event(br#"data:{"token": {"text": "tight"}}"#)],
        false,
        ErrorPolicy::Propagate,
    );

    assert_eq!(token(&items[0]).text, "tight");
}

#[test]
fn only_the_first_data_prefix_is_removed() {
    let (lines, guard) = capture();
    let items = sync_items(
        vec![event(br#"data: data: {"token": {"text": "twice"}}"#)],
        false,
        ErrorPolicy::Propagate,
    );
    drop(guard);

    assert!(matches!(items.as_slice(), [Ok(None)]));
    assert!(
        lines.text().contains(
            "ERROR Warning: Unparseable JSON data remained: data: {\"token\": {\"text\": \"twice\"}}"
        ),
        "{}",
        lines.text()
    );
}

#[test]
fn an_empty_payload_yields_nothing() {
    let items = sync_items(
        vec![event(b""), event(br#"{"token": {"text": "after"}}"#)],
        false,
        ErrorPolicy::Propagate,
    );

    assert_eq!(items.len(), 1);
    assert_eq!(token(&items[0]).text, "after");
}

#[test]
fn an_exception_frame_ends_a_propagating_stream() {
    let items = sync_items(
        vec![
            event(br#"{"token": {"text": "first"}}"#),
            exception(br#"{"message": "boom"}"#),
            event(br#"{"token": {"text": "third"}}"#),
        ],
        false,
        ErrorPolicy::Propagate,
    );

    assert_eq!(items.len(), 2);
    assert_eq!(token(&items[0]).text, "first");
    assert_eq!(
        items[1].as_ref().unwrap_err().to_string(),
        "Bad response code, expected 200: {\"message\": \"boom\"}"
    );
}

#[test]
fn an_exception_frame_is_skipped_when_asked_and_logged_as_an_error() {
    let (lines, guard) = capture();
    let items = sync_items(
        vec![
            event(br#"{"token": {"text": "first"}}"#),
            exception(br#"{"message": "boom"}"#),
            event(br#"{"token": {"text": "third"}}"#),
        ],
        false,
        ErrorPolicy::Skip,
    );
    drop(guard);

    assert_eq!(items.len(), 2);
    assert_eq!(token(&items[0]).text, "first");
    assert_eq!(token(&items[1]).text, "third");
    assert!(
        lines.text().starts_with(
            "ERROR Error parsing message: Bad response code, expected 200: {\"message\": \"boom\"}"
        ),
        "{}",
        lines.text()
    );
}

#[test]
fn invalid_utf8_ends_a_propagating_stream() {
    let items = sync_items(
        vec![
            event(&[0xff, 0xfe]),
            event(br#"{"token": {"text": "after"}}"#),
        ],
        false,
        ErrorPolicy::Propagate,
    );

    assert_eq!(items.len(), 1);
    assert!(matches!(
        items[0].as_ref().unwrap_err(),
        crate::sagemaker_stream::error::SagemakerStreamError::Utf8(_)
    ));
}

#[test]
fn invalid_utf8_is_skipped_with_a_warning() {
    let (lines, guard) = capture();
    let items = sync_items(
        vec![
            event(&[0xff, 0xfe]),
            event(br#"{"token": {"text": "after"}}"#),
        ],
        false,
        ErrorPolicy::Skip,
    );
    drop(guard);

    assert_eq!(items.len(), 1);
    assert_eq!(token(&items[0]).text, "after");
    assert!(
        lines.text().starts_with("WARN UnicodeDecodeError:"),
        "{}",
        lines.text()
    );
}

#[rstest]
#[case::propagate(ErrorPolicy::Propagate)]
#[case::skip(ErrorPolicy::Skip)]
fn a_frame_that_fails_its_checksum_ends_both_policies(#[case] policy: ErrorPolicy) {
    let chunk = corrupted(&event(br#"{"token": {"text": "lost"}}"#));
    let items = sync_items(vec![chunk], false, policy);

    assert_eq!(items.len(), 1);
    assert!(matches!(
        items[0].as_ref().unwrap_err(),
        crate::sagemaker_stream::error::SagemakerStreamError::Frame(_)
    ));
}

#[test]
fn a_projection_failure_keeps_the_buffer_for_the_next_event() {
    let (lines, guard) = capture();
    let items = sync_items(
        vec![
            event(br#"{"token": 5}"#),
            event(br#"{"token": {"text": "second"}}"#),
        ],
        false,
        ErrorPolicy::Skip,
    );
    drop(guard);

    assert!(matches!(items.as_slice(), [Ok(None)]));
    assert!(
        lines.text().contains("does not have the declared shape"),
        "{}",
        lines.text()
    );
}

#[test]
fn a_projection_failure_ends_a_propagating_stream() {
    let items = sync_items(
        vec![event(br#"{"index": "first"}"#)],
        false,
        ErrorPolicy::Propagate,
    );

    assert_eq!(items.len(), 1);
    assert!(matches!(
        items[0].as_ref().unwrap_err(),
        crate::sagemaker_stream::error::SagemakerStreamError::Json(_)
    ));
}

#[test]
fn a_null_token_text_reads_as_empty() {
    let items = sync_items(
        vec![event(br#"{"token": {"text": null}}"#)],
        false,
        ErrorPolicy::Propagate,
    );

    assert_eq!(token(&items[0]).text, "");
}

#[test]
fn a_final_projection_failure_is_logged_and_yields_nothing() {
    let (lines, guard) = capture();
    let items = sync_items(vec![event(br#"{"token": 5}"#)], false, ErrorPolicy::Skip);
    drop(guard);

    assert!(items.is_empty(), "{items:?}");
    assert!(
        lines.text().contains(
            "ERROR Final error parsing accumulated JSON: the SageMaker stream chunk does not have the declared shape"
        ),
        "{}",
        lines.text()
    );
}

// a caller that finishes after absorb reported the failure gets the same error from the buffer
#[test]
fn a_final_projection_failure_ends_a_propagating_stream() {
    let mut state = EventState {
        model: MODEL.to_owned(),
        is_messages_api: false,
        policy: ErrorPolicy::Propagate,
        frames: bytes::BytesMut::new(),
        accumulated: String::new(),
    };
    let mut failed = VecDeque::new();
    assert!(!state.absorb(&event(br#"{"token": 5}"#), &mut failed));
    let mut pending = VecDeque::new();
    state.finish(&mut pending);

    assert!(matches!(
        pending.pop_front(),
        Some(Err(SagemakerStreamError::Json(_)))
    ));
}

#[test]
fn a_messages_chunk_takes_the_decoder_model_and_the_chunk_object() {
    let items = sync_items(
        vec![event(
            br#"{"id": "1", "created": 2, "choices": [{"index": 0, "delta": {"content": "hi"}}]}"#,
        )],
        true,
        ErrorPolicy::Propagate,
    );

    let chunk = chat(&items[0]);
    assert_eq!(chunk.id, "1");
    assert_eq!(chunk.object, "chat.completion.chunk");
    assert_eq!(chunk.created, 2);
    assert_eq!(chunk.model, MODEL);
    assert_eq!(chunk.choices.len(), 1);
    assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hi"));
}

#[test]
fn a_messages_chunk_keeps_its_own_model_and_takes_the_chunk_object() {
    let items = sync_items(
        vec![event(
            br#"{"id": "1", "object": "other", "created": 2, "model": "own", "choices": []}"#,
        )],
        true,
        ErrorPolicy::Propagate,
    );

    let chunk = chat(&items[0]);
    assert_eq!(chunk.model, "own");
    assert_eq!(chunk.object, "chat.completion.chunk");
}

#[test]
fn a_messages_chunk_without_choices_ends_a_propagating_stream() {
    let items = sync_items(
        vec![event(br#"{"id": "1", "created": 2}"#), event(b"{}")],
        true,
        ErrorPolicy::Propagate,
    );

    assert_eq!(items.len(), 1);
    assert!(matches!(
        items[0].as_ref().unwrap_err(),
        crate::sagemaker_stream::error::SagemakerStreamError::Json(_)
    ));
}

#[test]
fn a_non_object_chunk_ends_a_propagating_stream() {
    let items = sync_items(vec![event(b"[1]")], false, ErrorPolicy::Propagate);

    assert_eq!(items.len(), 1);
    assert!(matches!(
        items[0].as_ref().unwrap_err(),
        crate::sagemaker_stream::error::SagemakerStreamError::Json(_)
    ));
}

#[rstest]
#[case::generic(false)]
#[case::messages(true)]
fn the_async_face_propagates_like_the_blocking_one(#[case] messages: bool) {
    let chunks = vec![
        event(br#"{"token": {"text": "first"}}"#),
        exception(br#"{"message": "boom"}"#),
        event(&[0xff]),
        event(br#"{"token": {"text": "after"}}"#),
    ];

    let propagated = async_items(chunks.clone(), messages, ErrorPolicy::Propagate);
    let blocking = sync_items(chunks, messages, ErrorPolicy::Propagate);
    assert_eq!(propagated.len(), blocking.len());
}

#[test]
fn the_async_face_skips_like_the_blocking_one() {
    let chunks = vec![
        event(br#"{"token": {"text": "first"}}"#),
        exception(br#"{"message": "boom"}"#),
        event(&[0xff]),
        event(br#"{"token": {"text": "after"}}"#),
    ];

    let items = async_items(chunks, false, ErrorPolicy::Skip);
    assert_eq!(items.len(), 2);
    assert_eq!(token(&items[0]).text, "first");
    assert_eq!(token(&items[1]).text, "after");
}

#[rstest]
#[case::propagate(ErrorPolicy::Propagate)]
#[case::skip(ErrorPolicy::Skip)]
fn a_source_error_ends_the_stream_in_both_policies(#[case] policy: ErrorPolicy) {
    let mut chunks: Vec<Result<Vec<u8>, HttpError>> = vec![Ok(event(b"{}"))];
    chunks.push(Err(HttpError::Timeout));
    let items: Vec<EventItem> = stream(chunks.into_iter(), false, policy).collect();

    assert_eq!(items.len(), 2);
    assert_eq!(token(&items[0]).index, Some(0));
    assert!(matches!(
        items[1].as_ref().unwrap_err(),
        crate::sagemaker_stream::error::SagemakerStreamError::Body(_)
    ));
}
