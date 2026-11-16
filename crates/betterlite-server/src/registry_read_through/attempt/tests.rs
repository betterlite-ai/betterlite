#![cfg(test)]

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::time::advance;

use crate::registry_read_through::tests::support::{CountingResync, FailingResync, capture_logs};
use crate::registry_read_through::{
    MAX_RECENT_MISSES, READ_THROUGH_MAX_RESYNCS, READ_THROUGH_MISS_TTL, READ_THROUGH_RESYNC_WINDOW,
    RegistryReadThrough, Resync, ResyncError,
};

#[test]
fn the_default_limits_match_litellm() {
    assert_eq!(READ_THROUGH_MISS_TTL, Duration::from_secs(2));
    assert_eq!(READ_THROUGH_RESYNC_WINDOW, Duration::from_secs(5));
    assert_eq!(READ_THROUGH_MAX_RESYNCS, 20);
    assert_eq!(MAX_RECENT_MISSES, 1000);
}

#[tokio::test(start_paused = true)]
async fn a_remembered_miss_is_answered_false_without_a_resync() {
    let calls = Arc::new(AtomicUsize::new(0));
    let read_through = RegistryReadThrough::new(CountingResync {
        calls: Arc::clone(&calls),
        found: false,
    });
    assert!(!read_through.attempt("ghost-model").await);
    assert!(!read_through.attempt("ghost-model").await);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_found_object_is_not_remembered() {
    let calls = Arc::new(AtomicUsize::new(0));
    let read_through = RegistryReadThrough::new(CountingResync {
        calls: Arc::clone(&calls),
        found: true,
    });
    assert!(read_through.attempt("new-model").await);
    assert!(read_through.attempt("new-model").await);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn a_miss_is_remembered_until_the_ttl_ends() {
    let calls = Arc::new(AtomicUsize::new(0));
    let read_through = RegistryReadThrough::new(CountingResync {
        calls: Arc::clone(&calls),
        found: false,
    });
    assert!(!read_through.attempt("ghost-model").await);
    advance(Duration::from_millis(1999)).await;
    assert!(!read_through.attempt("ghost-model").await);
    advance(Duration::from_millis(1)).await;
    assert!(!read_through.attempt("ghost-model").await);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    advance(Duration::from_millis(1)).await;
    assert!(!read_through.attempt("ghost-model").await);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn a_resync_error_is_logged_and_not_remembered() {
    let calls = Arc::new(AtomicUsize::new(0));
    let read_through = RegistryReadThrough::new(FailingResync {
        calls: Arc::clone(&calls),
        message: "the database is down",
    });
    let (log, _guard) = capture_logs();
    assert!(!read_through.attempt("ghost-model").await);
    assert!(!read_through.attempt("ghost-model").await);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let written = fs::read_to_string(log.path()).unwrap();
    assert!(written.contains("key=ghost-model"));
    assert!(written.contains("the database is down"));
}

#[tokio::test(start_paused = true)]
async fn the_recent_miss_map_keeps_at_most_a_thousand_keys() {
    let calls = Arc::new(AtomicUsize::new(0));
    let read_through = RegistryReadThrough::new(CountingResync {
        calls: Arc::clone(&calls),
        found: false,
    });
    remember_misses(&read_through, 1000).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1000);
    assert!(!read_through.attempt("missing-1000").await);
    assert_eq!(calls.load(Ordering::SeqCst), 1001);
    assert!(!read_through.attempt("missing-0000").await);
    assert_eq!(calls.load(Ordering::SeqCst), 1002);
    assert!(!read_through.attempt("missing-0500").await);
    assert_eq!(calls.load(Ordering::SeqCst), 1002);
}

#[tokio::test(start_paused = true)]
async fn two_tasks_that_ask_for_the_same_missing_key_run_the_resync_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let read_through = RegistryReadThrough::new(YieldingResync {
        calls: Arc::clone(&calls),
    });
    let (first, second) = tokio::join!(
        read_through.attempt("ghost-model"),
        read_through.attempt("ghost-model"),
    );
    assert!(!first);
    assert!(!second);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

async fn remember_misses(read_through: &RegistryReadThrough<CountingResync>, count: usize) {
    for index in 0..count {
        let key = format!("missing-{index:04}");
        assert!(!read_through.attempt(&key).await);
    }
}

struct YieldingResync {
    calls: Arc<AtomicUsize>,
}

impl Resync for YieldingResync {
    async fn resync(&self, _key: &str) -> Result<bool, ResyncError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::task::yield_now().await;
        Ok(false)
    }
}
