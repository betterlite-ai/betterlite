#![cfg(test)]

use std::error::Error as _;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{RouterSearchRefresh, SearchRefreshError};

#[derive(Debug, thiserror::Error)]
#[error("the search tools table is unreachable")]
struct TableUnreachable;

struct SucceedingRefresh {
    calls: AtomicUsize,
}

impl RouterSearchRefresh for SucceedingRefresh {
    fn refresh(&self) -> impl Future<Output = Result<(), SearchRefreshError>> + Send {
        self.calls.store(1, Ordering::SeqCst);
        ready(Ok(()))
    }
}

struct FailingRefresh {
    calls: AtomicUsize,
}

impl RouterSearchRefresh for FailingRefresh {
    fn refresh(&self) -> impl Future<Output = Result<(), SearchRefreshError>> + Send {
        self.calls.store(1, Ordering::SeqCst);
        ready(Err(SearchRefreshError {
            source: Box::new(TableUnreachable),
        }))
    }
}

#[tokio::test]
async fn refresh_succeeds_when_the_router_reloads() {
    let refresh = SucceedingRefresh {
        calls: AtomicUsize::new(0),
    };
    refresh.refresh().await.unwrap();
    assert_eq!(refresh.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refresh_reports_the_cause_of_a_failed_reload() {
    let refresh = FailingRefresh {
        calls: AtomicUsize::new(0),
    };
    let error = refresh.refresh().await.unwrap_err();
    assert_eq!(refresh.calls.load(Ordering::SeqCst), 1);
    assert_eq!(error.to_string(), "the search tools table is unreachable");
}

#[test]
fn error_displays_the_cause_text() {
    let error = SearchRefreshError {
        source: Box::new(TableUnreachable),
    };
    assert_eq!(error.to_string(), "the search tools table is unreachable");
}

#[test]
fn error_source_returns_the_cause() {
    let error = SearchRefreshError {
        source: Box::new(TableUnreachable),
    };
    let source = error.source().unwrap();
    assert_eq!(source.to_string(), "the search tools table is unreachable");
    assert!(source.downcast_ref::<TableUnreachable>().is_some());
}
