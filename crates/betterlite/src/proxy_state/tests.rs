#![cfg(test)]

use std::sync::atomic::Ordering;

use super::ProxyState;

#[test]
fn starts_at_zero() {
    let state = ProxyState::default();
    assert_eq!(state.spend_logs_row_count.load(Ordering::SeqCst), 0);
}

#[test]
fn set_then_get_round_trips() {
    let state = ProxyState::default();
    state.spend_logs_row_count.store(42, Ordering::SeqCst);
    assert_eq!(state.spend_logs_row_count.load(Ordering::SeqCst), 42);
}
