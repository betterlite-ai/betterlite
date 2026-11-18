#![cfg(test)]

use std::future::pending;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::http::request::Parts;
use betterlite_models::model_registration::JsonObject;
use futures_util::StreamExt as _;
use http::Request;
use tokio::runtime::Runtime;

use super::super::types::{SseDataGenerator, StreamSource};
use super::{
    EndState, Outcome, body_of, body_with_request, chat_chunk, frames, other_failure, quiet_source,
    recorded_outcomes, recording_generator, request, snapshot_handle, source,
};

fn recording_cleanup() -> (SseDataGenerator<String>, Arc<Mutex<Vec<Outcome>>>) {
    let outcome_log = Arc::new(Mutex::new(Vec::new()));
    let (generator, _hooks) = recording_generator(
        Arc::new(Mutex::new(Vec::new())),
        Arc::clone(&outcome_log),
        None,
    );
    (generator, outcome_log)
}

#[test]
fn cleanup_after_normal_end() {
    let (generator, outcome_log) = recording_cleanup();
    let mut stream_source = source(vec![Ok(chat_chunk("x"))], JsonObject::new());
    stream_source.snapshot = Some(snapshot_handle());
    let out = frames(&generator, stream_source, request(&[]), false);
    assert_eq!(out.len(), 2);
    let outcomes = recorded_outcomes(&outcome_log);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].key, "key-1");
    assert_eq!(outcomes[0].state, EndState::Completed);
    assert!(outcomes[0].has_snapshot);
    assert!(!outcomes[0].has_request);
}

#[test]
fn cleanup_after_error_frame() {
    let (generator, outcome_log) = recording_cleanup();
    let out = frames(
        &generator,
        source(vec![Err(other_failure("boom"))], JsonObject::new()),
        request(&[]),
        false,
    );
    assert_eq!(out.len(), 1);
    let outcomes = recorded_outcomes(&outcome_log);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].state, EndState::Completed);
    assert!(!outcomes[0].has_snapshot);
}

#[test]
fn cleanup_after_client_disconnect() {
    let (generator, outcome_log) = recording_cleanup();
    let mut body = body_of(
        &generator,
        quiet_source(vec![Ok(chat_chunk("x"))], JsonObject::new()),
        request(&[]),
        false,
    );
    let runtime = Runtime::new().expect("runtime");
    let first = runtime.block_on(body.next()).expect("frame");
    assert!(first.is_ok());
    drop(body);
    drop(runtime);
    let outcomes = recorded_outcomes(&outcome_log);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].state, EndState::Disconnected);
}

#[test]
fn cleanup_keeps_request_parts() {
    let (generator, outcome_log) = recording_cleanup();
    let parts: Parts = Request::builder()
        .uri("/v1/chat/completions")
        .body(())
        .expect("request")
        .into_parts()
        .0;
    let body = body_with_request(
        &generator,
        source(Vec::new(), JsonObject::new()),
        request(&[]),
        Some(parts),
        false,
    );
    let out = Runtime::new()
        .expect("runtime")
        .block_on(body.collect::<Vec<_>>());
    assert_eq!(out.len(), 1);
    let outcomes = recorded_outcomes(&outcome_log);
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].has_request);
    assert_eq!(outcomes[0].key, "key-1");
}

#[tokio::test]
async fn cleanup_waits_for_final_frame() {
    let (generator, outcome_log) = recording_cleanup();
    let mut body = body_of(
        &generator,
        source(Vec::new(), JsonObject::new()),
        request(&[]),
        false,
    );
    assert_eq!(
        body.next().await.expect("done").expect("frame"),
        "data: [DONE]\n\n"
    );
    assert!(recorded_outcomes(&outcome_log).is_empty());
    drop(body);
    assert_eq!(
        recorded_outcomes(&outcome_log)[0].state,
        EndState::Completed
    );
    assert_eq!(recorded_outcomes(&outcome_log).len(), 1);
}

#[tokio::test]
async fn cleanup_waits_for_error_frame() {
    let (generator, outcome_log) = recording_cleanup();
    let mut body = body_of(
        &generator,
        source(vec![Err(other_failure("boom"))], JsonObject::new()),
        request(&[]),
        false,
    );
    assert!(body.next().await.expect("error frame").is_ok());
    assert!(recorded_outcomes(&outcome_log).is_empty());
    assert!(body.next().await.is_none());
    assert_eq!(
        recorded_outcomes(&outcome_log)[0].state,
        EndState::Completed
    );
    assert_eq!(recorded_outcomes(&outcome_log).len(), 1);
}

struct UpstreamDrop(Arc<AtomicBool>);

impl Drop for UpstreamDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn dropping_body_drops_upstream() {
    let (generator, outcome_log) = recording_cleanup();
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = UpstreamDrop(Arc::clone(&dropped));
    let items = async_stream::stream! {
        let _guard = guard;
        yield Ok(chat_chunk("x"));
        pending::<()>().await;
    }
    .boxed();
    let mut body = body_of(
        &generator,
        StreamSource {
            items,
            hidden: JsonObject::new(),
            snapshot: None,
        },
        request(&[]),
        false,
    );
    assert!(body.next().await.expect("chunk").is_ok());
    assert!(!dropped.load(Ordering::SeqCst));
    drop(body);
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(
        recorded_outcomes(&outcome_log)[0].state,
        EndState::Disconnected
    );
}
