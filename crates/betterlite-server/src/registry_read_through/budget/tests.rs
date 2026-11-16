#![cfg(test)]

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::time::advance;

use crate::registry_read_through::tests::support::{CountingResync, FailingResync, capture_logs};
use crate::registry_read_through::{
    READ_THROUGH_MAX_RESYNCS, READ_THROUGH_RESYNC_WINDOW, RegistryReadThrough, Resync, State,
};

#[tokio::test(start_paused = true)]
async fn a_window_grants_twenty_resyncs() {
    let mut state = State::default();
    let granted = spend_resync_budget(&mut state, READ_THROUGH_MAX_RESYNCS);
    assert!(granted.iter().all(|flag| *flag));
    assert_eq!(granted.len(), READ_THROUGH_MAX_RESYNCS);
}

#[tokio::test(start_paused = true)]
async fn the_twenty_first_resync_in_a_window_is_refused() {
    let mut state = State::default();
    let granted = spend_resync_budget(&mut state, READ_THROUGH_MAX_RESYNCS);
    assert!(granted.iter().all(|flag| *flag));
    assert!(!state.consume_resync_budget(READ_THROUGH_MAX_RESYNCS, READ_THROUGH_RESYNC_WINDOW));
    assert!(!state.consume_resync_budget(READ_THROUGH_MAX_RESYNCS, READ_THROUGH_RESYNC_WINDOW));
}

#[tokio::test(start_paused = true)]
async fn a_new_window_restores_the_budget() {
    let mut state = State::default();
    spend_resync_budget(&mut state, READ_THROUGH_MAX_RESYNCS);
    assert!(!state.consume_resync_budget(READ_THROUGH_MAX_RESYNCS, READ_THROUGH_RESYNC_WINDOW));
    advance(READ_THROUGH_RESYNC_WINDOW).await;
    let granted = spend_resync_budget(&mut state, READ_THROUGH_MAX_RESYNCS);
    assert!(granted.iter().all(|flag| *flag));
    assert_eq!(granted.len(), READ_THROUGH_MAX_RESYNCS);
}

#[tokio::test(start_paused = true)]
async fn the_window_does_not_reset_one_millisecond_before_it_ends() {
    let mut state = State::default();
    spend_resync_budget(&mut state, READ_THROUGH_MAX_RESYNCS);
    advance(Duration::from_millis(4999)).await;
    assert!(!state.consume_resync_budget(READ_THROUGH_MAX_RESYNCS, READ_THROUGH_RESYNC_WINDOW));
    advance(Duration::from_millis(1)).await;
    assert!(state.consume_resync_budget(READ_THROUGH_MAX_RESYNCS, READ_THROUGH_RESYNC_WINDOW));
}

#[tokio::test(start_paused = true)]
async fn a_budget_refusal_logs_a_warning_and_does_not_remember_the_miss() {
    let calls = Arc::new(AtomicUsize::new(0));
    let read_through = RegistryReadThrough::new(CountingResync {
        calls: Arc::clone(&calls),
        found: false,
    });
    exhaust_budget(&read_through).await;
    assert_eq!(calls.load(Ordering::SeqCst), READ_THROUGH_MAX_RESYNCS);
    let (log, _guard) = capture_logs();
    assert!(!read_through.attempt("late-model").await);
    assert!(!read_through.attempt("late-model").await);
    assert_eq!(calls.load(Ordering::SeqCst), READ_THROUGH_MAX_RESYNCS);
    let written = fs::read_to_string(log.path()).unwrap();
    assert_eq!(written.matches("resync budget is exhausted").count(), 2);
    assert!(written.contains("key=late-model"));
    assert!(written.contains("max_resyncs=20"));
    assert!(written.contains("window_seconds=5"));
    advance(READ_THROUGH_RESYNC_WINDOW).await;
    assert!(!read_through.attempt("late-model").await);
    assert_eq!(calls.load(Ordering::SeqCst), 21);
}

#[tokio::test(start_paused = true)]
async fn a_resync_error_still_spends_a_budget_slot() {
    let calls = Arc::new(AtomicUsize::new(0));
    let read_through = RegistryReadThrough::new(FailingResync {
        calls: Arc::clone(&calls),
        message: "the database is down",
    });
    exhaust_budget(&read_through).await;
    assert_eq!(calls.load(Ordering::SeqCst), READ_THROUGH_MAX_RESYNCS);
    assert!(!read_through.attempt("late-model").await);
    assert_eq!(calls.load(Ordering::SeqCst), READ_THROUGH_MAX_RESYNCS);
}

fn spend_resync_budget(state: &mut State, attempts: usize) -> Vec<bool> {
    let mut granted = Vec::with_capacity(attempts);
    for _ in 0..attempts {
        granted.push(
            state.consume_resync_budget(READ_THROUGH_MAX_RESYNCS, READ_THROUGH_RESYNC_WINDOW),
        );
    }
    granted
}

async fn exhaust_budget<R>(read_through: &RegistryReadThrough<R>)
where
    R: Resync,
{
    for index in 0..READ_THROUGH_MAX_RESYNCS {
        let key = format!("missing-model-{index}");
        assert!(!read_through.attempt(&key).await);
    }
}
