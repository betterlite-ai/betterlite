#![cfg(test)]

use std::iter::repeat_with;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::future::join_all;

use super::shared::{NO_OVERRIDES, analyzer, find_secret, settings, spawn_mock};
use crate::presidio_analysis::CHUNK_CONCURRENCY;

const SLOW: Duration = Duration::from_millis(100);

#[tokio::test]
async fn concurrency_keeps_at_most_eight_chunk_requests_in_flight() {
    let mock = spawn_mock(find_secret, SLOW).await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 4;
    analyzer(configured)
        .analyze_text(&"a".repeat(60), &NO_OVERRIDES)
        .await
        .unwrap();
    assert!(mock.seen.lock().unwrap().len() > CHUNK_CONCURRENCY);
    assert_eq!(
        mock.peak_in_flight.load(Ordering::SeqCst),
        CHUNK_CONCURRENCY
    );
}

#[tokio::test]
async fn concurrency_bound_is_shared_by_concurrent_calls_on_one_analyzer() {
    let mock = spawn_mock(find_secret, SLOW).await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 4;
    let analyzer = analyzer(configured);
    let text = "a".repeat(40);
    let calls = repeat_with(|| analyzer.analyze_text(&text, &NO_OVERRIDES)).take(3);
    assert!(join_all(calls).await.iter().all(Result::is_ok));
    assert_eq!(
        mock.peak_in_flight.load(Ordering::SeqCst),
        CHUNK_CONCURRENCY
    );
}

#[tokio::test]
async fn concurrency_runs_fewer_chunks_than_the_bound_all_at_once() {
    let mock = spawn_mock(find_secret, SLOW).await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 20;
    analyzer(configured)
        .analyze_text(&"a".repeat(50), &NO_OVERRIDES)
        .await
        .unwrap();
    assert_eq!(mock.seen.lock().unwrap().len(), 4);
    assert_eq!(mock.peak_in_flight.load(Ordering::SeqCst), 4);
}
