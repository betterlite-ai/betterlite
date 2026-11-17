#![cfg(test)]

use std::error::Error;
use std::fmt::Debug;

use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_models::decisions::Json;
use rstest::rstest;
use tracing::Level;

use crate::api::messages_sse_events::stream::MessagesProviderStream;

use super::MessagesStreamChunk;
use super::MessagesStreamRestampInput;
use super::cache_stream::MessagesStreamCacheWriter;
use super::replay::{CachedMessagesResult, convert_cached_messages_result};
use super::restamp::{MAX_HELD_BYTES, is_ping_frame};
use super::restamp_messages_stream;

mod branches;
mod support;

use support::{
    NOT_STORED, REQUESTED_MODEL, Read, RecordingLogging, ScriptedStream, cache_deps,
    cache_settings, cached_dict, limits, payload_events, request_kwargs, tracker,
};

pub(super) fn text(value: &str) -> MessagesStreamChunk {
    MessagesStreamChunk::Text(value.to_owned())
}

pub(super) fn bytes(value: &[u8]) -> MessagesStreamChunk {
    MessagesStreamChunk::Bytes(value.to_vec())
}

pub(super) fn event(value: Json) -> MessagesStreamChunk {
    MessagesStreamChunk::Event(value)
}

pub(super) fn start_event_text(model: &str) -> String {
    format!("{{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_1\",\"model\":\"{model}\"}}}}")
}

pub(super) fn start_event(model: &str) -> Json {
    serde_json::from_str(&start_event_text(model)).expect("the fixture is JSON")
}

pub(super) fn start_frame(model: &str) -> String {
    format!(
        "event: message_start\ndata: {}\n\n",
        start_event_text(model)
    )
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture is JSON")
}

fn ping_event() -> Json {
    json("{\"type\":\"ping\"}")
}

fn ping_frame() -> String {
    "event: ping\ndata: {\"type\": \"ping\"}\n\n".to_owned()
}

fn delta_frame() -> String {
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0}\n\n"
        .to_owned()
}

struct RestampCase {
    chunks: Vec<MessagesStreamChunk>,
    expected: Vec<MessagesStreamChunk>,
    flushed: MessagesStreamChunk,
}

fn restamp_case(
    chunks: Vec<MessagesStreamChunk>,
    expected: Vec<MessagesStreamChunk>,
    flushed: MessagesStreamChunk,
) -> RestampCase {
    RestampCase {
        chunks,
        expected,
        flushed,
    }
}

fn split_across_chunks() -> RestampCase {
    let frame = start_frame("upstream-model");
    let at = frame
        .find("\"model\"")
        .map(|index| index.saturating_add(3))
        .expect("the fixture names the model");
    let (head, tail) = frame.split_at(at);
    restamp_case(
        vec![bytes(head.as_bytes()), bytes(tail.as_bytes())],
        vec![bytes(b""), bytes(start_frame(REQUESTED_MODEL).as_bytes())],
        bytes(b""),
    )
}

fn without_a_model() -> RestampCase {
    let source = "{\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\"}}";
    restamp_case(
        vec![event(json(source))],
        vec![event(start_event(REQUESTED_MODEL))],
        bytes(b""),
    )
}

fn message_is_not_an_object() -> RestampCase {
    let source = "{\"type\":\"message_start\",\"message\":\"claude-3\"}";
    restamp_case(
        vec![event(json(source))],
        vec![event(json(source))],
        bytes(b""),
    )
}

#[rstest]
#[case::whole_frame(restamp_case(
    vec![text(&start_frame("upstream-model"))],
    vec![text(&start_frame(REQUESTED_MODEL))],
    bytes(b""),
))]
#[case::split_across_chunks(split_across_chunks())]
#[case::already_the_requested_model(restamp_case(
    vec![text(&start_frame(REQUESTED_MODEL))],
    vec![text(&start_frame(REQUESTED_MODEL))],
    bytes(b""),
))]
#[case::event_without_a_model(without_a_model())]
#[case::message_is_not_an_object(message_is_not_an_object())]
#[case::data_line_without_message_start(restamp_case(
    vec![text(&delta_frame())],
    vec![text(&delta_frame())],
    bytes(b""),
))]
fn split_message_start_model(#[case] case: RestampCase) {
    let input = MessagesStreamRestampInput {
        requested_model: REQUESTED_MODEL.to_owned(),
        chunks: case.chunks,
    };
    let result = restamp_messages_stream(&input);
    assert_eq!(result.chunks, case.expected);
    assert_eq!(result.flushed, case.flushed);
}

#[rstest]
#[case::ping_block_keeps_the_stream_armed(restamp_case(
    vec![text(&ping_frame()), text(&start_frame("upstream-model"))],
    vec![text(&ping_frame()), text(&start_frame(REQUESTED_MODEL))],
    bytes(b""),
))]
#[case::compact_ping_marker_keeps_the_stream_armed(restamp_case(
    vec![
        text("event: ping\ndata: {\"type\":\"ping\"}\n\n"),
        text(&start_frame("upstream-model")),
    ],
    vec![
        text("event: ping\ndata: {\"type\":\"ping\"}\n\n"),
        text(&start_frame(REQUESTED_MODEL)),
    ],
    bytes(b""),
))]
#[case::event_ping_does_not_disarm(restamp_case(
    vec![event(ping_event()), event(start_event("upstream-model"))],
    vec![event(ping_event()), event(start_event(REQUESTED_MODEL))],
    bytes(b""),
))]
#[case::event_without_a_type_does_not_disarm(restamp_case(
    vec![event(json("{\"message\":{}}")), event(start_event("upstream-model"))],
    vec![
        event(json("{\"message\":{}}")),
        event(start_event(REQUESTED_MODEL)),
    ],
    bytes(b""),
))]
#[case::data_frame_disarms_the_stream(restamp_case(
    vec![text(&delta_frame()), text(&start_frame("upstream-model"))],
    vec![text(&delta_frame()), text(&start_frame("upstream-model"))],
    bytes(b""),
))]
#[case::message_start_disarms_the_stream(restamp_case(
    vec![
        text(&start_frame("upstream-model")),
        text(&start_frame("upstream-model")),
    ],
    vec![
        text(&start_frame(REQUESTED_MODEL)),
        text(&start_frame("upstream-model")),
    ],
    bytes(b""),
))]
fn ping_frame(#[case] case: RestampCase) {
    let input = MessagesStreamRestampInput {
        requested_model: REQUESTED_MODEL.to_owned(),
        chunks: case.chunks,
    };
    let result = restamp_messages_stream(&input);
    assert_eq!(result.chunks, case.expected);
    assert_eq!(result.flushed, case.flushed);
}

#[rstest]
#[case::event_line(b"event: ping", true)]
#[case::spaced_type(b"x \"type\": \"ping\" y", true)]
#[case::compact_type(b"x \"type\":\"ping\" y", true)]
#[case::message_start(b"event: message_start", false)]
#[case::empty(b"", false)]
fn ping_marker_matching(#[case] frame: &[u8], #[case] expected: bool) {
    assert_eq!(is_ping_frame(frame), expected);
}

fn held_data_line() -> RestampCase {
    restamp_case(
        vec![text(&format!(
            "data: {}\n",
            start_event_text("upstream-model")
        ))],
        vec![bytes(b"")],
        bytes(format!("data: {}\n", start_event_text(REQUESTED_MODEL)).as_bytes()),
    )
}

fn filler(width: usize) -> RestampCase {
    let filler = vec![b'x'; width];
    restamp_case(vec![bytes(&filler)], vec![bytes(b"")], bytes(&filler))
}

fn oversized_filler() -> RestampCase {
    let width = MAX_HELD_BYTES.saturating_add(1);
    let filler = vec![b'x'; width];
    restamp_case(vec![bytes(&filler)], vec![bytes(&filler)], bytes(b""))
}

#[rstest]
#[case::held_data_line_is_flushed(held_data_line())]
#[case::held_bytes_without_the_event_come_back(restamp_case(
    vec![bytes(b"event: ping\n")],
    vec![bytes(b"")],
    bytes(b"event: ping\n"),
))]
#[case::the_limit_is_held(filler(MAX_HELD_BYTES))]
#[case::past_the_limit_is_emitted(oversized_filler())]
#[case::nothing_was_held(restamp_case(Vec::new(), Vec::new(), bytes(b"")))]
fn partial_final_frame_flush(#[case] case: RestampCase) {
    let input = MessagesStreamRestampInput {
        requested_model: REQUESTED_MODEL.to_owned(),
        chunks: case.chunks,
    };
    let result = restamp_messages_stream(&input);
    assert_eq!(result.chunks, case.expected);
    assert_eq!(result.flushed, case.flushed);
}

pub(super) fn sse_event(event_type: &str, data: &str) -> String {
    format!("event: {event_type}\ndata: {data}\n\n")
}

pub(super) fn cache_stream_frames() -> Vec<String> {
    vec![
        sse_event(
            "message_start",
            "{\"type\":\"message_start\",\"message\":{\"model\":\"claude-3\"}}",
        ),
        sse_event(
            "content_block_delta",
            "{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}",
        ),
        sse_event("message_stop", "{\"type\":\"message_stop\"}"),
    ]
}

fn split_reads(frames: &[String]) -> Vec<Read> {
    let second = frames.get(1).expect("the fixture has a middle frame");
    let at = second
        .find("delta")
        .expect("the middle frame names the delta");
    let (head, tail) = second.split_at(at);
    let first = frames.first().expect("the fixture has a first frame");
    let last = frames.get(2).expect("the fixture has a last frame");
    vec![
        Read::Chunk(format!("{first}{head}").into_bytes()),
        Read::Chunk(format!("{tail}{last}").into_bytes()),
    ]
}

pub(super) async fn drain_writer<S>(writer: &mut MessagesStreamCacheWriter<S>) -> Vec<Vec<u8>>
where
    S: MessagesProviderStream + Send,
    S::Error: Error + Debug + 'static,
{
    let mut yielded = Vec::new();
    while let Some(chunk) = writer
        .next_chunk()
        .await
        .expect("the writer relays the stream")
    {
        yielded.push(chunk);
    }
    yielded
}

pub(super) async fn written_stream(
    reads: Vec<Read>,
    preset: Option<&str>,
) -> (Vec<Vec<u8>>, Json, KeyObject) {
    let cache = support::RecordingCache::succeeding();
    let tracker = tracker();
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(reads),
        cache_settings(request_kwargs(), preset),
        cache_deps(Some(cache.clone()), &tracker, limits(64)),
    );
    let yielded = drain_writer(&mut writer).await;
    tracker.wait_idle().await;
    let mut writes = cache.writes();
    assert_eq!(writes.len(), 1, "{NOT_STORED}");
    let (payload, kwargs) = writes.pop().expect("the write was recorded");
    (yielded, payload, kwargs)
}

#[tokio::test]
async fn cache_stream_replay() {
    let frames = cache_stream_frames();
    let joined = frames.concat();
    let (yielded, payload, kwargs) = written_stream(split_reads(&frames), Some("preset-key")).await;
    assert_eq!(yielded, vec![joined.into_bytes()]);
    assert_eq!(payload_events(&payload), frames);
    assert_eq!(
        kwargs.get("cache_key"),
        Some(&KeyValue::String("preset-key".to_owned()))
    );

    let logging = RecordingLogging::default();
    let converted = convert_cached_messages_result(cached_dict(&payload), logging.clone())
        .expect("the cache replays");
    let CachedMessagesResult::Stream(mut iterator) = converted else {
        panic!("the cached payload replays as a stream");
    };
    let mut replayed = Vec::new();
    while let Some(chunk) = iterator.next_chunk().await.expect("the replay never fails") {
        replayed.push(chunk);
    }
    let expected: Vec<Vec<u8>> = frames
        .iter()
        .map(|frame| frame.clone().into_bytes())
        .collect();
    assert_eq!(replayed, expected);
    assert!(logging.stamped());
    assert_eq!(logging.billed(), vec![replayed]);
    assert!(
        iterator
            .next_chunk()
            .await
            .expect("the replay ends")
            .is_none()
    );
    assert!(logging.billed().is_empty());
}

#[tokio::test]
async fn cache_write_failure_warning() {
    let (guard, captured) = support::capture();
    let cache = support::RecordingCache::failing("the cache backend is down");
    let tracker = tracker();
    let frames = cache_stream_frames();
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(vec![Read::Chunk(frames.concat().into_bytes())]),
        cache_settings(request_kwargs(), None),
        cache_deps(Some(cache.clone()), &tracker, limits(64)),
    );
    let yielded = drain_writer(&mut writer).await;
    tracker.wait_idle().await;
    drop(guard);

    assert_eq!(yielded, vec![frames.concat().into_bytes()]);
    assert_eq!(cache.writes().len(), 1, "{NOT_STORED}");
    let events = captured.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].level, Level::ERROR);
    assert!(
        events[0]
            .message
            .contains("Anthropic Messages stream cache write failed")
    );
}
