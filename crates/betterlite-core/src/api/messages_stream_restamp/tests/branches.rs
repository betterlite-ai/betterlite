#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::PoisonError;

use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::replay::CachedDict;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::Level;

use crate::api::api_boundaries::ApiLimits;

use super::super::CACHED_STREAM_EVENTS_KEY;
use super::super::MessagesStreamRestampInput;
use super::super::cache_stream::{
    MessagesStreamCacheError, MessagesStreamCacheSettings, MessagesStreamCacheWriter,
};
use super::super::replay::{
    CachedMessagesResult, CachedStreamEventsError, convert_cached_messages_result,
    get_cached_stream_events,
};
use super::super::restamp::{StreamModelRestamper, restamp_chunk};
use super::super::restamp_messages_stream;
use super::support::{
    REQUESTED_MODEL, Read, RecordingCache, RecordingLogging, ReplayHarness, ScriptedStream,
    cache_deps, cache_hit, cache_settings, cached_events_dict, limits, request_kwargs, tracker,
};
use super::{bytes, cache_stream_frames, drain_writer, event, sse_event, start_frame, text};
use crate::api::messages_sse_events::stream::MessagesProviderStream as _;

fn settings_without_support() -> MessagesStreamCacheSettings {
    MessagesStreamCacheSettings {
        supported_call_types: None,
        ..cache_settings(request_kwargs(), None)
    }
}

fn no_store_kwargs() -> KeyObject {
    let mut kwargs = request_kwargs();
    kwargs.insert(
        "cache".to_owned(),
        KeyValue::Object(IndexMap::from([(
            "no-store".to_owned(),
            KeyValue::Bool(true),
        )])),
    );
    kwargs
}

async fn written(
    cache: Option<Arc<RecordingCache>>,
    reads: Vec<Read>,
    settings: MessagesStreamCacheSettings,
    api_limits: ApiLimits,
) -> (Vec<Vec<u8>>, usize) {
    let tracker = tracker();
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(reads),
        settings,
        cache_deps(cache.clone(), &tracker, api_limits),
    );
    let yielded = drain_writer(&mut writer).await;
    tracker.wait_idle().await;
    let write_count = cache.map_or(0, |recording| recording.writes().len());
    (yielded, write_count)
}

fn every_frame_as_a_read() -> Vec<Read> {
    cache_stream_frames()
        .iter()
        .map(|frame| Read::Chunk(frame.clone().into_bytes()))
        .collect()
}

fn writer_limits(max_items: usize, max_json_bytes: usize) -> ApiLimits {
    ApiLimits {
        max_json_bytes,
        max_event_bytes: ApiLimits::default().max_event_bytes,
        max_items,
    }
}

fn huge_delta_frame() -> String {
    let filler = "x".repeat(1024 * 1024 + 2048);
    sse_event(
        "content_block_delta",
        &format!(
            "{{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{filler}\"}}}}"
        ),
    )
}

#[tokio::test]
async fn writer_without_a_cache_still_relays() {
    let joined = cache_stream_frames().concat();
    let (yielded, writes) = written(
        None,
        vec![Read::Chunk(joined.clone().into_bytes())],
        cache_settings(request_kwargs(), None),
        limits(64),
    )
    .await;
    assert_eq!(yielded, vec![joined.into_bytes()]);
    assert_eq!(writes, 0);
}

#[tokio::test]
async fn writer_skips_an_incomplete_stream() {
    let frames = cache_stream_frames();
    let (_, writes) = written(
        Some(RecordingCache::succeeding()),
        vec![Read::Chunk(frames[0].clone().into_bytes())],
        cache_settings(request_kwargs(), None),
        limits(64),
    )
    .await;
    assert_eq!(writes, 0);
}

#[tokio::test]
async fn writer_skips_a_provider_error_stream() {
    let frames = cache_stream_frames();
    let error = sse_event(
        "error",
        "{\"type\":\"error\",\"error\":{\"type\":\"api_error\",\"message\":\"boom\"}}",
    );
    let reads = vec![Read::Chunk(
        format!("{}{error}{}", frames[0], frames[2]).into_bytes(),
    )];
    let (_, writes) = written(
        Some(RecordingCache::succeeding()),
        reads,
        cache_settings(request_kwargs(), None),
        limits(64),
    )
    .await;
    assert_eq!(writes, 0);
}

#[tokio::test]
async fn writer_skips_unsupported_call_types() {
    let (_, writes) = written(
        Some(RecordingCache::succeeding()),
        every_frame_as_a_read(),
        settings_without_support(),
        limits(64),
    )
    .await;
    assert_eq!(writes, 0);
}

#[tokio::test]
async fn writer_skips_a_no_store_request() {
    let (_, writes) = written(
        Some(RecordingCache::succeeding()),
        every_frame_as_a_read(),
        cache_settings(no_store_kwargs(), None),
        limits(64),
    )
    .await;
    assert_eq!(writes, 0);
}

#[tokio::test]
async fn writer_skips_an_oversized_stream() {
    let (_, writes) = written(
        Some(RecordingCache::succeeding()),
        every_frame_as_a_read(),
        cache_settings(request_kwargs(), None),
        limits(1),
    )
    .await;
    assert_eq!(writes, 0);
}

#[tokio::test]
async fn writer_keeps_a_stream_at_the_item_limit() {
    let (_, writes) = written(
        Some(RecordingCache::succeeding()),
        every_frame_as_a_read(),
        cache_settings(request_kwargs(), None),
        writer_limits(3, ApiLimits::default().max_json_bytes),
    )
    .await;
    assert_eq!(writes, 1);
}

#[tokio::test]
async fn writer_drops_a_stream_past_the_item_limit() {
    let (_, writes) = written(
        Some(RecordingCache::succeeding()),
        every_frame_as_a_read(),
        cache_settings(request_kwargs(), None),
        writer_limits(2, ApiLimits::default().max_json_bytes),
    )
    .await;
    assert_eq!(writes, 0);
}

#[tokio::test]
async fn writer_keeps_a_stream_at_the_byte_limit() {
    let joined = cache_stream_frames().concat();
    let (_, writes) = written(
        Some(RecordingCache::succeeding()),
        every_frame_as_a_read(),
        cache_settings(request_kwargs(), None),
        writer_limits(64, joined.len()),
    )
    .await;
    assert_eq!(writes, 1);
}

#[tokio::test]
async fn writer_drops_a_stream_past_the_byte_limit() {
    let joined = cache_stream_frames().concat();
    let (_, writes) = written(
        Some(RecordingCache::succeeding()),
        every_frame_as_a_read(),
        cache_settings(request_kwargs(), None),
        writer_limits(64, joined.len().saturating_sub(1)),
    )
    .await;
    assert_eq!(writes, 0);
}

#[tokio::test]
async fn writer_scans_the_joined_stream_under_the_payload_limit() {
    let frames = cache_stream_frames();
    let reads = vec![
        Read::Chunk(frames[0].clone().into_bytes()),
        Read::Chunk(huge_delta_frame().into_bytes()),
        Read::Chunk(frames[2].clone().into_bytes()),
    ];
    let (yielded, writes) = written(
        Some(RecordingCache::succeeding()),
        reads,
        cache_settings(request_kwargs(), None),
        writer_limits(64, 2 * 1024 * 1024),
    )
    .await;
    assert_eq!(yielded.len(), 3);
    assert_eq!(writes, 1);
}

#[tokio::test]
async fn writer_surfaces_a_malformed_cache_control() {
    let mut kwargs = request_kwargs();
    kwargs.insert("cache".to_owned(), KeyValue::String("on".to_owned()));
    let frames = cache_stream_frames();
    let tracker = tracker();
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(vec![Read::Chunk(frames.concat().into_bytes())]),
        cache_settings(kwargs, None),
        cache_deps(Some(RecordingCache::succeeding()), &tracker, limits(64)),
    );
    let first = writer.next_chunk().await.expect("the first chunk relays");
    assert!(first.is_some());
    let error = writer
        .next_chunk()
        .await
        .expect_err("the malformed cache control surfaces");
    assert!(matches!(error, MessagesStreamCacheError::Store(_)));
}

#[tokio::test]
async fn writer_forwards_close_and_buffered_output() {
    let stream = ScriptedStream::new(Vec::new()).with_buffered_output(Json::Bool(true));
    let tracker = tracker();
    let mut writer = MessagesStreamCacheWriter::new(
        stream,
        cache_settings(request_kwargs(), None),
        cache_deps(None, &tracker, limits(64)),
    );
    assert!(writer.has_buffered_provider_output());
    assert_eq!(writer.buffered_provider_output(), Some(Json::Bool(true)));
    assert!(matches!(writer.aclose().await, Some(Ok(()))));

    let mut quiet = MessagesStreamCacheWriter::new(
        ScriptedStream::new(Vec::new()).with_buffered_output(Json::Text("yes".to_owned())),
        cache_settings(request_kwargs(), None),
        cache_deps(None, &tracker, limits(64)),
    );
    assert!(!quiet.has_buffered_provider_output());
    assert_eq!(quiet.buffered_provider_output(), None);
    assert!(matches!(quiet.aclose().await, Some(Ok(()))));
}

#[tokio::test]
async fn writer_propagates_a_stream_failure() {
    let tracker = tracker();
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(vec![Read::Failure("upstream died")]),
        cache_settings(request_kwargs(), None),
        cache_deps(None, &tracker, limits(64)),
    );
    let error = writer
        .next_chunk()
        .await
        .expect_err("the stream failure surfaces");
    assert!(matches!(error, MessagesStreamCacheError::Stream(_)));
    assert_eq!(error.to_string(), "upstream died");
}

#[tokio::test]
async fn writer_logs_an_invalid_utf8_stream() {
    let (guard, captured) = super::support::capture();
    let cache = RecordingCache::succeeding();
    let tracker = tracker();
    let mut payload = sse_event("message_stop", "{\"type\":\"message_stop\"}").into_bytes();
    payload.push(0xff);
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(vec![Read::Chunk(payload)]),
        cache_settings(request_kwargs(), None),
        cache_deps(Some(Arc::clone(&cache)), &tracker, limits(64)),
    );
    let yielded = drain_writer(&mut writer).await;
    tracker.wait_idle().await;
    drop(guard);

    assert_eq!(yielded.len(), 1);
    assert_eq!(cache.writes().len(), 0);
    let events = captured.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].level, Level::ERROR);
    assert!(
        events[0]
            .message
            .contains("Anthropic Messages stream cache write failed")
    );
}

#[tokio::test]
async fn writer_maps_a_close_failure() {
    let tracker = tracker();
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(Vec::new()).with_close_failure("the client went away"),
        cache_settings(request_kwargs(), None),
        cache_deps(None, &tracker, limits(64)),
    );
    let Some(Err(error)) = writer.aclose().await else {
        panic!("the close failure surfaces");
    };
    assert!(matches!(error, MessagesStreamCacheError::Stream(_)));
    assert_eq!(error.to_string(), "the client went away");
}

#[tokio::test]
async fn writer_persists_once() {
    let cache = RecordingCache::succeeding();
    let tracker = tracker();
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(every_frame_as_a_read()),
        cache_settings(request_kwargs(), None),
        cache_deps(Some(Arc::clone(&cache)), &tracker, limits(64)),
    );
    let yielded = drain_writer(&mut writer).await;
    assert!(
        writer
            .next_chunk()
            .await
            .expect("the second end relays")
            .is_none()
    );
    tracker.wait_idle().await;
    assert_eq!(yielded.len(), 3);
    assert_eq!(cache.writes().len(), 1);
}

#[test]
fn replay_without_the_events_key_stays_unchanged() {
    let cached = BTreeMap::from([("other".to_owned(), CacheValue::Text("value".to_owned()))]);
    let logging = RecordingLogging::default();
    let converted =
        convert_cached_messages_result(cached, logging.clone()).expect("no events, no error");
    assert!(matches!(converted, CachedMessagesResult::Unchanged(_)));
    assert!(!logging.stamped());

    let wrong_type = BTreeMap::from([(
        CACHED_STREAM_EVENTS_KEY.to_owned(),
        CacheValue::Text("not a list".to_owned()),
    )]);
    let unchanged = convert_cached_messages_result(wrong_type, logging.clone())
        .expect("a non-list value carries no events");
    assert!(matches!(unchanged, CachedMessagesResult::Unchanged(_)));
    assert!(!logging.stamped());
}

#[test]
fn cached_events_keep_text_and_bytes_only() {
    let cached: CachedDict = BTreeMap::from([(
        CACHED_STREAM_EVENTS_KEY.to_owned(),
        CacheValue::Array(vec![
            CacheValue::Bytes(b"event: ping\n\n".to_vec()),
            CacheValue::Text("event: ping\n\n".to_owned()),
            CacheValue::Integer(7),
            CacheValue::Object(BTreeMap::new()),
        ]),
    )]);
    let events = get_cached_stream_events(&cached)
        .expect("the events are UTF-8")
        .expect("the key carries events");
    assert_eq!(
        events,
        vec!["event: ping\n\n".to_owned(), "event: ping\n\n".to_owned()]
    );
}

#[test]
fn cached_bytes_must_be_utf8() {
    let cached: CachedDict = BTreeMap::from([(
        CACHED_STREAM_EVENTS_KEY.to_owned(),
        CacheValue::Array(vec![CacheValue::Bytes(vec![0xff, 0xfe])]),
    )]);
    let error = get_cached_stream_events(&cached).expect_err("invalid UTF-8 is an error");
    assert!(matches!(
        error,
        CachedStreamEventsError::Utf8 { index: 0, .. }
    ));
}

#[tokio::test]
async fn replay_logging_stamps_cache_hit_and_bills() {
    let harness = ReplayHarness::new();
    let events = cache_stream_frames();
    let converted = convert_cached_messages_result(cached_events_dict(&events), harness.replay)
        .expect("the cached events replay");
    assert_eq!(cache_hit(&harness.logging_obj), Some(Json::Bool(true)));
    let CachedMessagesResult::Stream(mut iterator) = converted else {
        panic!("the cached events replay as a stream");
    };
    let mut replayed = Vec::new();
    while let Some(chunk) = iterator.next_chunk().await.expect("the replay never fails") {
        replayed.push(chunk);
    }
    let expected: Vec<Vec<u8>> = events
        .iter()
        .map(|event| event.as_bytes().to_vec())
        .collect();
    assert_eq!(replayed, expected);
    let routed = harness
        .seams
        .routed
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].url_route, "/v1/messages");
    assert_eq!(routed[0].endpoint_type, EndpointType::Anthropic);
    assert_eq!(routed[0].raw_bytes, expected);
}

#[tokio::test]
async fn replay_iterator_ends_after_the_last_event() {
    let events = vec![cache_stream_frames()[0].clone()];
    let CachedMessagesResult::Stream(mut iterator) =
        convert_cached_messages_result(cached_events_dict(&events), RecordingLogging::default())
            .expect("the cached events replay")
    else {
        panic!("the cached events replay as a stream");
    };
    assert_eq!(
        iterator.next_chunk().await.expect("the replay never fails"),
        Some(events[0].clone().into_bytes())
    );
    assert_eq!(iterator.next_chunk().await.expect("the replay ends"), None);
}

#[test]
fn flush_disarms_the_stream() {
    let mut restamper = StreamModelRestamper::new(REQUESTED_MODEL.to_owned());
    assert_eq!(restamper.flush(), bytes(b""));
    let later = restamper.process(&text(&start_frame("upstream-model")));
    assert_eq!(later, text(&start_frame("upstream-model")));
}

#[rstest]
#[case::done_payload("event: message_start\ndata: [DONE]\n\n")]
#[case::empty_payload("event: message_start\ndata:   \n\n")]
#[case::invalid_json("event: message_start\ndata: {oops\n\n")]
#[case::non_object_payload("event: message_start\ndata: [1,2]\n\n")]
#[case::no_data_line("event: message_start\n\n")]
fn restamp_leaves_unrestampable_frames_alone(#[case] frame: &str) {
    let input = MessagesStreamRestampInput {
        requested_model: REQUESTED_MODEL.to_owned(),
        chunks: vec![text(frame)],
    };
    let result = restamp_messages_stream(&input);
    assert_eq!(result.chunks, vec![bytes(frame.as_bytes())]);
    assert_eq!(result.flushed, bytes(b""));
}

#[test]
fn restamp_chunk_keeps_bytes_without_the_marker() {
    let chunk = bytes(b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n");
    assert_eq!(restamp_chunk(&chunk, REQUESTED_MODEL), chunk);
}

#[test]
fn restamp_chunk_restamps_bytes_with_the_marker() {
    let frame = start_frame("upstream-model");
    assert_eq!(
        restamp_chunk(&bytes(frame.as_bytes()), REQUESTED_MODEL),
        bytes(start_frame(REQUESTED_MODEL).as_bytes())
    );
}

#[test]
fn restamp_chunk_keeps_text_frames_text() {
    let delta = text("event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n");
    assert_eq!(restamp_chunk(&delta, REQUESTED_MODEL), delta);
    let done = text("event: message_start\ndata: [DONE]\n\n");
    assert_eq!(restamp_chunk(&done, REQUESTED_MODEL), done);
}

#[test]
fn restamp_chunk_restamps_a_text_frame() {
    let frame = start_frame("upstream-model");
    assert_eq!(
        restamp_chunk(&text(&frame), REQUESTED_MODEL),
        text(&start_frame(REQUESTED_MODEL))
    );
}

#[test]
fn restamp_chunk_keeps_events_that_are_not_objects() {
    let chunk = event(Json::Text("claude-3".to_owned()));
    assert_eq!(restamp_chunk(&chunk, REQUESTED_MODEL), chunk);
}
