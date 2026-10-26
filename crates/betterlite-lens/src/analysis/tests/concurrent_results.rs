#![cfg(test)]

use super::primitives::concurrent_results;
use super::{ANALYSIS_CONCURRENCY, AnalysisError};
use std::future::pending;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Semaphore;

#[tokio::test]
async fn default_concurrency() {
    assert_eq!(ANALYSIS_CONCURRENCY, 8);
    let calls = AtomicUsize::new(0);
    let mut results = concurrent_results(
        (0_u32..10).collect(),
        |value| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move { Ok(value) }
        },
        None,
    )
    .unwrap();
    results.next().await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 8);
    let mut count = 1_u32;
    while results.next().await.is_some() {
        count += 1;
    }
    assert_eq!(count, 10);
}

#[tokio::test]
async fn completion_order_and_drop() {
    let started = Arc::new(Semaphore::new(0));
    let held = Arc::new(Semaphore::new(1));
    let start_signal = Arc::clone(&started);
    let hold_signal = Arc::clone(&held);
    let mut results = concurrent_results(
        vec![0_u32, 1],
        move |value| blocked_operation(value, Arc::clone(&start_signal), Arc::clone(&hold_signal)),
        Some(2),
    )
    .unwrap();
    assert_eq!(results.next().await.unwrap().unwrap(), 1);
    drop(results);
    let _released = held.acquire().await.unwrap();
}

#[tokio::test]
async fn error_stops_remaining_work() {
    let mut results = concurrent_results(
        vec![0_u32, 1],
        |_| async { Err::<(), _>(AnalysisError::Callback(String::from("failed"))) },
        Some(1),
    )
    .unwrap();
    results.next().await.unwrap().unwrap_err();
    assert!(results.next().await.is_none());
    assert!(matches!(
        concurrent_results(vec![0_u32], |_| async { Ok(()) }, Some(0)),
        Err(AnalysisError::Validation(_))
    ));
}

async fn blocked_operation(
    value: u32,
    start: Arc<Semaphore>,
    hold: Arc<Semaphore>,
) -> Result<u32, AnalysisError> {
    if value == 0 {
        let _permit = hold.acquire().await.unwrap();
        start.add_permits(1);
        pending::<()>().await;
    }
    start.acquire().await.unwrap().forget();
    Ok(value)
}
