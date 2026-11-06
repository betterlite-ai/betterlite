#![cfg(test)]

use std::sync::Arc;

use tokio::runtime::Builder;

use super::shared::{analyzer, settings};
use crate::presidio_analysis::CHUNK_CONCURRENCY;
use crate::presidio_analysis::error::PresidioAnalysisError;

#[tokio::test]
async fn chunk_semaphore_is_shared_by_calls_on_the_same_runtime() {
    let analyzer = analyzer(settings("http://unused"));
    let first = analyzer.chunk_semaphore().unwrap();
    let second = analyzer.chunk_semaphore().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.available_permits(), CHUNK_CONCURRENCY);
}

#[test]
fn chunk_semaphore_is_separate_for_each_runtime() {
    let analyzer = analyzer(settings("http://unused"));
    let first_runtime = Builder::new_current_thread().build().unwrap();
    let second_runtime = Builder::new_current_thread().build().unwrap();
    let first = first_runtime
        .block_on(async { analyzer.chunk_semaphore() })
        .unwrap();
    let again = first_runtime
        .block_on(async { analyzer.chunk_semaphore() })
        .unwrap();
    let second = second_runtime
        .block_on(async { analyzer.chunk_semaphore() })
        .unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(analyzer.chunk_semaphores.lock().unwrap().len(), 2);
}

#[test]
fn chunk_semaphore_needs_a_running_runtime() {
    let analyzer = analyzer(settings("http://unused"));
    let error = analyzer.chunk_semaphore().unwrap_err();
    assert!(matches!(error, PresidioAnalysisError::NoRuntime { .. }));
    assert_eq!(error.status(), 500);
}
