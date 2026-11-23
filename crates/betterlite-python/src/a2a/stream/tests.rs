#![cfg(test)]

use std::future::{pending, ready};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_core::agents::json::AgentJson;
use tokio::sync::mpsc::error::SendError;
use tokio::sync::{Mutex, Notify, mpsc};

use super::{Chunk, Delivery, Feed, build_stream, chunk_of, start, stop_stream, take_chunk};
use crate::a2a::convert::Call;
use crate::a2a::error::A2aCallError;

#[test]
fn closing_a_stream_marks_it_closed_for_aclose_and_drop() {
    let (_explicit_sender, explicit_receiver) = mpsc::channel(1);
    let explicit = build_stream(explicit_receiver, Box::pin(pending::<Delivery>()));
    stop_stream(&explicit);
    assert!(explicit.closed.load(Ordering::SeqCst));
    assert!(explicit.feed.try_lock().unwrap().driver.is_none());
    let (_dropped_sender, dropped_receiver) = mpsc::channel(1);
    let dropped = build_stream(dropped_receiver, Box::pin(pending::<Delivery>()));
    let closed = Arc::clone(&dropped.closed);
    drop(dropped);
    assert!(closed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn take_chunk_hands_back_a_queued_chunk() {
    let (sender, receiver) = mpsc::channel(1);
    sender.send(Ok(String::from("one"))).await.unwrap();
    let feed = Arc::new(Mutex::new(Feed {
        receiver,
        driver: Some(Box::pin(ready(Ok::<(), SendError<Chunk>>(())))),
    }));
    let cancel = Arc::new(Notify::new());
    let closed = Arc::new(AtomicBool::new(false));
    match take_chunk(feed, cancel, closed).await {
        Some(Ok(text)) => assert_eq!(text, "one"),
        _ => panic!("take_chunk handed back no chunk"),
    }
}

#[test]
fn start_rejects_a_call_without_a_request() {
    let error = start(None, Call::default()).err().unwrap();
    assert!(matches!(error, A2aCallError::MissingRequest));
}

#[test]
fn chunk_of_writes_the_event_as_json() {
    let event: AgentJson = serde_json::from_str(r#"{"kind":"message"}"#).unwrap();
    match chunk_of(Ok(event)) {
        Ok(text) => assert_eq!(text, r#"{"kind":"message"}"#),
        _ => panic!("a plain event failed to encode"),
    }
}
