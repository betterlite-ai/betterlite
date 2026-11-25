#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{into_future, run};
use tokio_util::task::TaskTracker;

use super::awaitable;
use crate::cache_backends::error::BackendHandleError;

// The work runs inside the owner's tracker, answers None, and reports its failure as a RuntimeError.
#[test]
fn blocking_cache_work_runs_and_reports_errors() {
    Python::initialize();
    let tasks = TaskTracker::new();
    let observer = tasks.clone();
    let tracked = Arc::new(AtomicUsize::new(0));
    let written = Arc::clone(&tracked);
    let work = async move {
        let success = Python::attach(|attached| {
            into_future(awaitable(attached, tasks.clone(), move || {
                written.store(observer.len(), Ordering::SeqCst);
                Ok::<(), BackendHandleError>(())
            })?)
        })?;
        let answer = success.await?;
        Python::attach(|attached| assert!(answer.is_none(attached)));
        let failure = Python::attach(|attached| {
            into_future(awaitable(attached, tasks.clone(), || {
                Err(BackendHandleError::IncrementUnavailable)
            })?)
        })?;
        assert_eq!(
            failure.await.unwrap_err().to_string(),
            "RuntimeError: the increment result is not available"
        );
        assert!(tasks.is_empty());
        Ok(())
    };
    Python::attach(|py| run(py, work)).unwrap();
    assert_eq!(tracked.load(Ordering::SeqCst), 1_usize);
}
