#![cfg(test)]

use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::super::RelayLimits;
use super::super::registry::StreamRegistry;
use super::support::{Frame, Harness, at, frame, read, settle};
use crate::api::messages_sse_events::render::incomplete_error_sse_event;
use crate::api::messages_stream_logging::SseChunk;

fn limits() -> RelayLimits {
    RelayLimits {
        queue_maxsize: 8,
        max_detached_stream_drains: 100,
    }
}

fn mapping(pairs: Vec<(&str, Json)>) -> SseChunk {
    SseChunk::Mapping(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

#[tokio::test]
async fn normal_terminal_and_incomplete() {
    forwards_raw_bytes_and_encodes_dicts().await;
    a_raw_message_stop_suppresses_the_incomplete_event().await;
    a_mapping_message_stop_suppresses_the_incomplete_event().await;
    an_error_body_suppresses_the_incomplete_event().await;
    eof_emits_the_incomplete_event_without_billing_it().await;
    the_first_chunk_stamps_the_completion_time_once().await;
}

async fn forwards_raw_bytes_and_encodes_dicts() {
    let (harness, mut relay) = Harness::start(limits(), Arc::new(StreamRegistry::default()));
    let ping = "event: ping\ndata: {\"type\": \"ping\"}\n\n";
    harness.script.release(SseChunk::Bytes(frame(ping))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk(ping.to_owned()));
    let delta = mapping(vec![
        ("type", Json::Text("content_block_delta".to_owned())),
        (
            "delta",
            Json::Object(IndexMap::from([
                ("type".to_owned(), Json::Text("text_delta".to_owned())),
                ("text".to_owned(), Json::Text("hi".to_owned())),
            ])),
        ),
    ]);
    harness.script.release(delta).await;
    match read(&mut relay).await {
        Frame::Chunk(encoded) => {
            assert!(encoded.starts_with("event: content_block_delta\ndata: {"));
            assert!(encoded.contains("\"text\": \"hi\""));
        }
        other => panic!("the mapping is encoded, got {other:?}"),
    }
    let untyped = mapping(vec![("content", Json::Text("x".to_owned()))]);
    harness.script.release(untyped).await;
    match read(&mut relay).await {
        Frame::Chunk(encoded) => assert!(encoded.starts_with("event: message\ndata: {")),
        other => panic!("the mapping without a type becomes a message event, got {other:?}"),
    }
    harness.script.finish().await;
    let incomplete = String::from_utf8(incomplete_error_sse_event()).expect("the event is UTF-8");
    assert_eq!(read(&mut relay).await, Frame::Chunk(incomplete));
    assert_eq!(read(&mut relay).await, Frame::End);
    settle(relay).await;
}

async fn a_raw_message_stop_suppresses_the_incomplete_event() {
    let (harness, mut relay) = Harness::start(limits(), Arc::new(StreamRegistry::default()));
    let stop = "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    harness.script.release(SseChunk::Bytes(frame(stop))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk(stop.to_owned()));
    harness.script.finish().await;
    assert_eq!(read(&mut relay).await, Frame::End);
    assert!(relay.reached_end);
    harness.worker.flush().await;
    assert_eq!(harness.ran(), 1);
    settle(relay).await;
}

async fn a_mapping_message_stop_suppresses_the_incomplete_event() {
    let (harness, mut relay) = Harness::start(limits(), Arc::new(StreamRegistry::default()));
    harness
        .script
        .release(mapping(vec![(
            "type",
            Json::Text("message_stop".to_owned()),
        )]))
        .await;
    match read(&mut relay).await {
        Frame::Chunk(encoded) => assert!(encoded.starts_with("event: message_stop\ndata: ")),
        other => panic!("the stop mapping forwards, got {other:?}"),
    }
    harness.script.finish().await;
    assert_eq!(read(&mut relay).await, Frame::End);
    settle(relay).await;
}

async fn an_error_body_suppresses_the_incomplete_event() {
    let (harness, mut relay) = Harness::start(limits(), Arc::new(StreamRegistry::default()));
    let error = mapping(vec![
        ("type", Json::Text("error".to_owned())),
        (
            "error",
            Json::Object(IndexMap::from([
                ("type".to_owned(), Json::Text("overloaded_error".to_owned())),
                ("message".to_owned(), Json::Text("busy".to_owned())),
            ])),
        ),
    ]);
    harness.script.release(error).await;
    match read(&mut relay).await {
        Frame::Chunk(encoded) => {
            assert!(encoded.starts_with("event: error\ndata: {"));
            assert!(encoded.contains("\"overloaded_error\""));
        }
        other => panic!("the error body forwards, got {other:?}"),
    }
    harness.script.finish().await;
    assert_eq!(read(&mut relay).await, Frame::End);
    settle(relay).await;
}

async fn eof_emits_the_incomplete_event_without_billing_it() {
    let (harness, mut relay) = Harness::start(limits(), Arc::new(StreamRegistry::default()));
    let chunk = "data: {\"ok\": true}\n\n";
    harness.script.release(SseChunk::Bytes(frame(chunk))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk(chunk.to_owned()));
    assert!(!relay.reached_end);
    harness.script.finish().await;
    let incomplete = String::from_utf8(incomplete_error_sse_event()).expect("the event is UTF-8");
    assert_eq!(read(&mut relay).await, Frame::Chunk(incomplete));
    assert!(!relay.reached_end);
    assert_eq!(read(&mut relay).await, Frame::End);
    assert!(relay.reached_end);
    harness.worker.flush().await;
    assert_eq!(harness.routed_frames(), vec![frame(chunk)]);
    settle(relay).await;
}

async fn the_first_chunk_stamps_the_completion_time_once() {
    let (harness, mut relay) = Harness::start(limits(), Arc::new(StreamRegistry::default()));
    let ping = "event: ping\ndata: {\"type\": \"ping\"}\n\n";
    harness.script.release(SseChunk::Bytes(frame(ping))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk(ping.to_owned()));
    harness.script.release(SseChunk::Bytes(frame(ping))).await;
    assert_eq!(read(&mut relay).await, Frame::Chunk(ping.to_owned()));
    harness.script.finish().await;
    settle(relay).await;
    harness.worker.flush().await;
    // the teardown and pump copies read the clock first, so the first chunk stamps at(2)
    assert_eq!(harness.completion_start_time(), Some(at(2)));
}
