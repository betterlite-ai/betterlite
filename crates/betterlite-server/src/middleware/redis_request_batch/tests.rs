#![cfg(test)]

mod support;

use std::io;
use std::str;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::to_bytes;
use betterlite_redis::request_batch::scope::active_request_redis_batch;
use tokio::time::timeout;
use tower::{Layer as _, Service as _};
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use self::support::{Inner, InnerFailed, Probe, Reply, take, unreachable_cache};
use super::RedisBatchLayer;

#[tokio::test]
async fn scope_is_active_inside_the_inner_service_and_closed_after() {
    let cache = unreachable_cache().await;
    let (handler, seen) = Inner::for_test(Arc::clone(&cache), Reply::Responded, false);
    let mut service = RedisBatchLayer.layer(handler);
    let response = service.call(support::request()).await.expect("a response");
    assert_eq!(response.status(), 200);
    assert_eq!(support::text(response).await, "ok");
    let batch = take(&seen);
    assert_eq!(batch.pending(), 0);
    assert_eq!(batch.flushes(), 0, "nothing was declared, so nothing ran");
    assert!(
        active_request_redis_batch(&cache).is_none(),
        "the scope is closed once the call returns"
    );
}

#[tokio::test]
async fn scope_is_closed_after_the_inner_service_fails() {
    let cache = unreachable_cache().await;
    let (handler, seen) = Inner::for_test(Arc::clone(&cache), Reply::Failed, false);
    let mut service = RedisBatchLayer.layer(handler);
    let error = service
        .call(support::request())
        .await
        .expect_err("the inner error");
    assert_eq!(error, InnerFailed, "the inner error is returned unchanged");
    assert_eq!(take(&seen).pending(), 0);
    assert!(
        active_request_redis_batch(&cache).is_none(),
        "the scope is closed once the call returns"
    );
}

#[tokio::test]
async fn scope_is_fresh_for_each_call() {
    let cache = unreachable_cache().await;
    let (handler, seen) = Inner::for_test(Arc::clone(&cache), Reply::Responded, false);
    let mut service = RedisBatchLayer.layer(handler);
    service.call(support::request()).await.expect("a response");
    let first = take(&seen);
    service.call(support::request()).await.expect("a response");
    let second = take(&seen);
    assert!(
        !Arc::ptr_eq(&first, &second),
        "a second call must not reuse the first call's batches"
    );
}

#[tokio::test]
async fn flush_runs_after_the_response() {
    let cache = unreachable_cache().await;
    let (handler, seen) = Inner::for_test(Arc::clone(&cache), Reply::Responded, true);
    let mut service = RedisBatchLayer.layer(handler);
    let response = service.call(support::request()).await.expect("a response");
    assert_eq!(response.status(), 200);
    assert_eq!(support::text(response).await, "ok");
    let batch = take(&seen);
    assert_eq!(
        batch.flushes(),
        1,
        "the pending declaration is flushed once"
    );
    assert_eq!(batch.pending(), 0, "nothing is left pending after the call");
}

#[tokio::test]
async fn flush_runs_after_an_inner_error() {
    let cache = unreachable_cache().await;
    let (handler, seen) = Inner::for_test(Arc::clone(&cache), Reply::Failed, true);
    let mut service = RedisBatchLayer.layer(handler);
    let error = service
        .call(support::request())
        .await
        .expect_err("the inner error");
    assert_eq!(error, InnerFailed, "the inner error is returned unchanged");
    let batch = take(&seen);
    assert_eq!(
        batch.flushes(),
        1,
        "the pending declaration is flushed despite the error"
    );
    assert_eq!(batch.pending(), 0);
}

#[tokio::test]
async fn flush_failure_is_logged_and_keeps_the_response() {
    let cache = unreachable_cache().await;
    let (handler, seen) = Inner::for_test(Arc::clone(&cache), Reply::Responded, true);
    let mut service = RedisBatchLayer.layer(handler);
    let records = Arc::new(Mutex::new(String::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(CaptureWriter(Arc::clone(&records)))
        .finish();
    let _guard = set_default(subscriber);
    let response = service.call(support::request()).await.expect("a response");
    assert_eq!(
        support::text(response).await,
        "ok",
        "a flush failure never changes the response"
    );
    assert_eq!(take(&seen).flushes(), 1, "the flush still runs");
    let logged = records.lock().expect("the log records").clone();
    assert!(logged.contains("WARN"), "{logged}");
    assert!(
        logged.contains("request_redis_batch: pipeline of 1 ops failed"),
        "{logged}"
    );
}

#[tokio::test]
async fn flush_runs_when_the_response_head_returns_not_when_the_body_ends() {
    let cache = unreachable_cache().await;
    let (handler, seen) = Inner::for_test(Arc::clone(&cache), Reply::Streaming, true);
    let mut service = RedisBatchLayer.layer(handler);
    let response = service
        .call(support::request())
        .await
        .expect("a response head");
    assert_eq!(
        take(&seen).flushes(),
        1,
        "the flush ran when the head returned"
    );
    let body = to_bytes(response.into_body(), 1024);
    let stalled = timeout(Duration::from_millis(50), body).await;
    assert!(stalled.is_err(), "the response body is still streaming");
}

#[tokio::test]
async fn nested_layers_share_the_outer_scope_batches() {
    let cache = unreachable_cache().await;
    let (handler, handler_seen) = Inner::for_test(Arc::clone(&cache), Reply::Responded, true);
    let inner_layer = RedisBatchLayer.layer(handler);
    let (probe, probe_seen) = Probe::for_test(Arc::clone(&cache), inner_layer);
    let mut service = RedisBatchLayer.layer(probe);
    let response = service.call(support::request()).await.expect("a response");
    assert_eq!(response.status(), 200);
    let outer = take(&probe_seen);
    let inner = take(&handler_seen);
    assert!(
        Arc::ptr_eq(&outer, &inner),
        "a second layer joins the outer scope instead of opening its own batches"
    );
    assert_eq!(
        outer.flushes(),
        1,
        "one shared batch means one pipeline for both declarations"
    );
    assert_eq!(outer.pending(), 0);
}

#[derive(Clone)]
struct CaptureWriter(Arc<Mutex<String>>);

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = str::from_utf8(buf)
            .map_err(|source| io::Error::new(io::ErrorKind::InvalidData, source))?;
        self.0
            .lock()
            .map_err(|source| io::Error::other(source.to_string()))?
            .push_str(text);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CaptureWriter {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}
