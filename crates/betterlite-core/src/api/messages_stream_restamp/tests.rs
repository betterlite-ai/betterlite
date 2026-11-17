#![cfg(test)]

use std::error::Error;
use std::fmt::Debug;
use std::sync::Arc;

use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_models::decisions::Json;
use rstest::rstest;
use tracing::Level;

use crate::api::messages_sse_events::stream::MessagesProviderStream;

use super::cache_stream::MessagesStreamCacheWriter;
use super::python_text::{decode_utf8_ignore, splitlines_keepends};
use super::replay::{CachedMessagesResult, convert_cached_messages_result};
use super::restamp::{MAX_HELD_BYTES, is_ping_frame};
use super::restamp_messages_stream;
use super::{CACHED_STREAM_EVENTS_KEY, MessagesStreamChunk, MessagesStreamRestampInput};

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

fn spaced_start_frame(model: &str) -> String {
    format!(
        "event: message_start\ndata: {{\"type\": \"message_start\", \"message\": {{\"id\": \"msg_1\", \"model\": \"{model}\"}}}}\n\n"
    )
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture is JSON")
}

fn ping_event() -> Json {
    json("{\"type\":\"ping\"}")
}

fn ping_sse_frame() -> String {
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

fn assert_restamp_case(case: RestampCase) {
    let input = MessagesStreamRestampInput {
        requested_model: REQUESTED_MODEL.to_owned(),
        chunks: case.chunks,
    };
    let result = restamp_messages_stream(&input);
    assert_eq!(result.chunks, case.expected);
    assert_eq!(result.flushed, case.flushed);
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

fn data_line(padding: &str, terminator: &str, emitted: bool) -> RestampCase {
    let upstream = format!(
        "{padding}data: {}{terminator}",
        start_event_text("upstream-model")
    );
    let restamped = format!("data: {}{terminator}", start_event_text(REQUESTED_MODEL));
    let (chunks, flushed) = if emitted {
        (vec![bytes(restamped.as_bytes())], bytes(b""))
    } else {
        (vec![bytes(b"")], bytes(restamped.as_bytes()))
    };
    restamp_case(vec![text(&upstream)], chunks, flushed)
}

fn indented_data_line() -> RestampCase {
    data_line("  ", "\n\n", true)
}

#[rstest]
#[case::whole_frame(restamp_case(
    vec![text(&start_frame("upstream-model"))],
    vec![bytes(start_frame(REQUESTED_MODEL).as_bytes())],
    bytes(b""),
))]
#[case::split_across_chunks(split_across_chunks())]
#[case::already_the_requested_model(restamp_case(
    vec![text(&spaced_start_frame(REQUESTED_MODEL))],
    vec![bytes(spaced_start_frame(REQUESTED_MODEL).as_bytes())],
    bytes(b""),
))]
#[case::event_without_a_model(without_a_model())]
#[case::message_is_not_an_object(message_is_not_an_object())]
#[case::data_line_without_message_start(restamp_case(
    vec![text(&delta_frame())],
    vec![bytes(delta_frame().as_bytes())],
    bytes(b""),
))]
#[case::data_line_with_leading_whitespace(indented_data_line())]
#[case::two_frames_in_one_read(restamp_case(
    vec![text(&format!("{}{}", ping_sse_frame(), start_frame("upstream-model")))],
    vec![bytes(
        format!("{}{}", ping_sse_frame(), start_frame(REQUESTED_MODEL)).as_bytes()
    )],
    bytes(b""),
))]
#[case::delta_mentioning_message_start_disarms(restamp_case(
    vec![
        text(
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"text\":\"message_start\"}\n\n"
        ),
        text(&start_frame("upstream-model")),
    ],
    vec![
        bytes(
            b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"text\":\"message_start\"}\n\n"
        ),
        text(&start_frame("upstream-model")),
    ],
    bytes(b""),
))]
fn split_message_start_model(#[case] case: RestampCase) {
    assert_restamp_case(case);
}

fn torn_after_ping() -> RestampCase {
    let torn = "event: content_block_delta\ndata: {\"type\":\"content_bl";
    let second = format!("{}{torn}", start_frame("upstream-model"));
    restamp_case(
        vec![text(&ping_sse_frame()), bytes(second.as_bytes())],
        vec![
            bytes(ping_sse_frame().as_bytes()),
            bytes(format!("{}{torn}", start_frame(REQUESTED_MODEL)).as_bytes()),
        ],
        bytes(b""),
    )
}

// The ping leaves the stream armed, so the torn start behind it is held and restamped once it closes
fn ping_block_then_torn_start() -> RestampCase {
    let frame = start_frame("upstream-model");
    let half = frame.len().checked_div(2).expect("2 is nonzero");
    let (torn, rest) = frame.split_at(half);
    restamp_case(
        vec![text(&format!("{}{torn}", ping_sse_frame())), text(rest)],
        vec![
            bytes(ping_sse_frame().as_bytes()),
            bytes(start_frame(REQUESTED_MODEL).as_bytes()),
        ],
        bytes(b""),
    )
}

#[rstest]
#[case::ping_block_keeps_the_stream_armed(restamp_case(
    vec![text(&ping_sse_frame()), text(&start_frame("upstream-model"))],
    vec![
        bytes(ping_sse_frame().as_bytes()),
        bytes(start_frame(REQUESTED_MODEL).as_bytes()),
    ],
    bytes(b""),
))]
#[case::compact_ping_marker_keeps_the_stream_armed(restamp_case(
    vec![
        text("event: ping\ndata: {\"type\":\"ping\"}\n\n"),
        text(&start_frame("upstream-model")),
    ],
    vec![
        bytes(b"event: ping\ndata: {\"type\":\"ping\"}\n\n"),
        bytes(start_frame(REQUESTED_MODEL).as_bytes()),
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
#[case::message_start_event_disarms_the_stream(restamp_case(
    vec![
        event(start_event("upstream-model")),
        event(start_event("upstream-model")),
    ],
    vec![
        event(start_event(REQUESTED_MODEL)),
        event(start_event("upstream-model")),
    ],
    bytes(b""),
))]
#[case::data_frame_disarms_the_stream(restamp_case(
    vec![text(&delta_frame()), text(&start_frame("upstream-model"))],
    vec![
        bytes(delta_frame().as_bytes()),
        text(&start_frame("upstream-model")),
    ],
    bytes(b""),
))]
#[case::message_start_disarms_the_stream(restamp_case(
    vec![
        text(&start_frame("upstream-model")),
        text(&start_frame("upstream-model")),
    ],
    vec![
        bytes(start_frame(REQUESTED_MODEL).as_bytes()),
        text(&start_frame("upstream-model")),
    ],
    bytes(b""),
))]
#[case::torn_after_ping(torn_after_ping())]
#[case::frame_without_data_keeps_the_stream_armed(restamp_case(
    vec![
        text("event: comment\n\n"),
        text(&start_frame("upstream-model")),
    ],
    vec![
        bytes(b"event: comment\n\n"),
        bytes(start_frame(REQUESTED_MODEL).as_bytes()),
    ],
    bytes(b""),
))]
#[case::ping_block_then_torn_start(ping_block_then_torn_start())]
fn ping_frame(#[case] case: RestampCase) {
    assert_restamp_case(case);
}

#[rstest]
#[case::number(json("{\"type\":1}"))]
#[case::boolean(json("{\"type\":true}"))]
#[case::array(json("{\"type\":[]}"))]
#[case::object(json("{\"type\":{}}"))]
fn non_text_event_types_disarm_the_stream(#[case] first: Json) {
    let input = MessagesStreamRestampInput {
        requested_model: REQUESTED_MODEL.to_owned(),
        chunks: vec![event(first.clone()), event(start_event("upstream-model"))],
    };
    let result = restamp_messages_stream(&input);
    assert_eq!(
        result.chunks,
        vec![event(first), event(start_event("upstream-model"))],
        "a non-text event type is neither absent nor ping, so the stream disarms"
    );
    assert_eq!(result.flushed, bytes(b""));
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

#[rstest]
#[case::crlf("\r\n")]
#[case::cr("\r")]
fn crlf_and_cr_frame_ends_restamp(#[case] line_end: &str) {
    let frame = format!(
        "event: message_start{line_end}data: {}{line_end}{line_end}",
        start_event_text("upstream-model")
    );
    let input = MessagesStreamRestampInput {
        requested_model: REQUESTED_MODEL.to_owned(),
        chunks: vec![text(&frame)],
    };
    let result = restamp_messages_stream(&input);
    assert_eq!(
        result.chunks,
        vec![bytes(
            format!(
                "event: message_start{line_end}data: {}{line_end}{line_end}",
                start_event_text(REQUESTED_MODEL)
            )
            .as_bytes()
        )],
        "{line_end:?} is one of LiteLLM's frame terminators"
    );
    assert_eq!(result.flushed, bytes(b""));
}

#[test]
fn splitlines_keeps_python_line_endings() {
    let source = "one\r\ntwo\nthree\rfour\u{b}five\u{c}six\u{1c}seven\u{1d}eight\u{1e}nine\u{85}ten\u{2028}eleven\u{2029}twelve";
    assert_eq!(
        splitlines_keepends(source),
        vec![
            "one\r\n",
            "two\n",
            "three\r",
            "four\u{b}",
            "five\u{c}",
            "six\u{1c}",
            "seven\u{1d}",
            "eight\u{1e}",
            "nine\u{85}",
            "ten\u{2028}",
            "eleven\u{2029}",
            "twelve",
        ]
    );
    assert_eq!(
        splitlines_keepends("one\n\ntwo"),
        vec!["one\n", "\n", "two"]
    );
    assert_eq!(splitlines_keepends("one\n"), vec!["one\n"]);
}

#[rstest]
#[case::non_breaking_space("\u{a0}")]
#[case::em_space("\u{2003}")]
#[case::ideographic_space("\u{3000}")]
#[case::unit_separator("\u{1f}")]
fn padded_data_line_strips_like_python(#[case] padding: &str) {
    let frame = format!("{padding}data: {}\n\n", start_event_text("upstream-model"));
    let input = MessagesStreamRestampInput {
        requested_model: REQUESTED_MODEL.to_owned(),
        chunks: vec![text(&frame)],
    };
    let result = restamp_messages_stream(&input);
    assert_eq!(
        result.chunks,
        vec![bytes(
            format!("data: {}\n\n", start_event_text(REQUESTED_MODEL)).as_bytes()
        )],
        "{padding:?} counts as whitespace for str.strip"
    );
}

#[test]
fn utf8_decoding_ignores_invalid_sequences() {
    assert_eq!(decode_utf8_ignore(b"a\xffb\xe2\x82"), "ab");
    assert_eq!(decode_utf8_ignore("h\u{e9}llo".as_bytes()), "h\u{e9}llo");
}

fn held_data_line() -> RestampCase {
    data_line("", "\n", false)
}

fn unterminated_data_line() -> RestampCase {
    data_line("", "", false)
}

fn filler(width: usize) -> RestampCase {
    let held = vec![b'x'; width];
    restamp_case(vec![bytes(&held)], vec![bytes(b"")], bytes(&held))
}

fn oversized_filler() -> RestampCase {
    let width = 0x0001_0001_usize;
    let held = vec![b'x'; width];
    restamp_case(vec![bytes(&held)], vec![bytes(&held)], bytes(b""))
}

#[rstest]
#[case::held_data_line_is_flushed(held_data_line())]
#[case::held_unterminated_data_line_is_flushed(unterminated_data_line())]
#[case::held_bytes_without_the_event_come_back(restamp_case(
    vec![bytes(b"event: ping\n")],
    vec![bytes(b"")],
    bytes(b"event: ping\n"),
))]
#[case::the_limit_is_held(filler(0x0001_0000))]
#[case::past_the_limit_is_emitted(oversized_filler())]
#[case::nothing_was_held(restamp_case(Vec::new(), Vec::new(), bytes(b"")))]
#[case::held_tail_without_the_marker_is_unchanged(restamp_case(
    vec![text(
        "data: {\"type\":\"message\\u005fstart\",\"message\":{\"id\":\"msg_1\",\"model\":\"upstream-model\"}}"
    )],
    vec![bytes(b"")],
    bytes(
        b"data: {\"type\":\"message\\u005fstart\",\"message\":{\"id\":\"msg_1\",\"model\":\"upstream-model\"}}"
    ),
))]
fn partial_final_frame_flush(#[case] case: RestampCase) {
    assert_eq!(MAX_HELD_BYTES, 0x0001_0000_usize);
    assert_restamp_case(case);
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

fn split_halves(frames: &[String]) -> (String, String) {
    let second = frames.get(1).expect("the fixture has a middle frame");
    let at = second
        .find("delta")
        .expect("the middle frame names the delta");
    let (head, tail) = second.split_at(at);
    let first = frames.first().expect("the fixture has a first frame");
    let last = frames.get(2).expect("the fixture has a last frame");
    (format!("{first}{head}"), format!("{tail}{last}"))
}

fn split_reads(frames: &[String]) -> Vec<Read> {
    let (head, tail) = split_halves(frames);
    vec![
        Read::Chunk(head.into_bytes()),
        Read::Chunk(tail.into_bytes()),
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

async fn written_stream(reads: Vec<Read>, preset: Option<&str>) -> (Vec<Vec<u8>>, Json, KeyObject) {
    let cache = support::RecordingCache::succeeding();
    let tracker = tracker();
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(reads),
        cache_settings(request_kwargs(), preset),
        cache_deps(Some(Arc::clone(&cache)), &tracker, limits(64)),
    );
    let yielded = drain_writer(&mut writer).await;
    tracker.wait_idle().await;
    let mut recorded = cache.writes();
    assert_eq!(recorded.len(), 1, "{NOT_STORED}");
    let (payload, kwargs) = recorded.pop().expect("the write was recorded");
    (yielded, payload, kwargs)
}

#[tokio::test]
async fn cache_stream_replay() {
    assert_eq!(
        CACHED_STREAM_EVENTS_KEY,
        "litellm_cached_anthropic_sse_events"
    );
    let frames = cache_stream_frames();
    let (head, tail) = split_halves(&frames);
    let (yielded, payload, kwargs) = written_stream(split_reads(&frames), Some("preset-key")).await;
    assert_eq!(yielded, vec![head.into_bytes(), tail.into_bytes()]);
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
        cache_deps(Some(Arc::clone(&cache)), &tracker, limits(64)),
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
