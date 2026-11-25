#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::{into_future, run};

use super::into_py;
use crate::cache_backends::error::BackendHandleError;

#[test]
fn blocking_cache_work_runs_and_reports_errors() {
    Python::initialize();
    let completed = Arc::new(AtomicBool::new(false));
    let written = Arc::clone(&completed);
    Python::attach(|py| {
        run(py, async move {
            let success = Python::attach(|attached| {
                into_future(into_py(attached, move || {
                    written.store(true, Ordering::SeqCst);
                    Ok::<(), BackendHandleError>(())
                })?)
            })?;
            let answer = success.await?;
            Python::attach(|attached| assert!(answer.is_none(attached)));
            let failure = Python::attach(|attached| {
                into_future(into_py(attached, || {
                    Err(BackendHandleError::IncrementUnavailable)
                })?)
            })?;
            assert_eq!(
                failure.await.unwrap_err().to_string(),
                "RuntimeError: the increment result is not available"
            );
            Ok(())
        })
        .unwrap();
    });
    assert!(completed.load(Ordering::SeqCst));
}
