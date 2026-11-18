#![cfg(test)]

use std::future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_router::redis_failure_policy::classify::FailureClass;

use crate::redis_cache_wiring::gate::classify;

use super::support::{
    Broken, FakeClock, bare_busy_loading_error, bare_cluster_down_error, bare_command_error,
    bare_connection_error, bare_timeout_error, busy_loading_error, chained_timeout_error,
    cluster_down_error, command_error, connect_timeout_error, connection_error, decode_error,
    gate_with, settings, status, timeout_error,
};

#[test]
fn a_connection_error_counts_as_a_hard_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = connection_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
    assert_eq!(observed.timeout_count, 0);
}

#[test]
fn a_timeout_counts_as_a_timeout() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = timeout_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.timeout_count, 1);
    assert_eq!(observed.hard_failure_count, 0);
}

#[test]
fn a_bare_redis_timeout_counts_as_a_timeout() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_timeout_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.timeout_count, 1);
    assert_eq!(observed.hard_failure_count, 0);
}

#[test]
fn a_bare_redis_connection_error_counts_as_a_hard_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_connection_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
    assert_eq!(observed.timeout_count, 0);
}

#[test]
fn a_refused_connection_classifies_as_a_connection_failure() {
    let error = io::Error::new(io::ErrorKind::ConnectionRefused, "Connection refused");
    let (class, causes) = classify(&error).expect("an io error classifies");
    assert_eq!(class, FailureClass::Connection);
    assert!(causes.is_empty());
}

#[test]
fn a_server_reply_classifies_as_a_command_failure() {
    let error = bare_command_error();
    let (class, _causes) = classify(&error).expect("the reply classifies");
    assert_eq!(class, FailureClass::Command);
}

#[test]
fn a_connection_error_chained_from_a_timeout_counts_as_a_timeout() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = chained_timeout_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.timeout_count, 1);
    assert_eq!(observed.hard_failure_count, 0);
}

#[test]
fn a_loading_server_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = busy_loading_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_cluster_down_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = cluster_down_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_bare_loading_server_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_busy_loading_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[test]
fn a_bare_cluster_down_reply_counts_as_a_health_failure() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = bare_cluster_down_error();
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}

#[tokio::test]
async fn a_connect_timeout_counts_as_a_timeout() {
    let elapsed = tokio::time::timeout(Duration::ZERO, future::pending::<()>())
        .await
        .expect_err("the timeout expires");
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = connect_timeout_error(elapsed);
    gate.record_failure(&error);
    let observed = status(&gate);
    assert_eq!(observed.timeout_count, 1);
    assert_eq!(observed.hard_failure_count, 0);
}

#[test]
fn a_command_error_changes_nothing() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = command_error();
    gate.record_failure(&error);
    assert_eq!(status(&gate).failure_count, 0);
    gate.enter("async_get_cache")
        .expect("the breaker stays closed");
}

#[test]
fn a_decode_error_changes_nothing() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = decode_error();
    gate.record_failure(&error);
    assert_eq!(status(&gate).failure_count, 0);
    gate.enter("async_get_cache")
        .expect("the breaker stays closed");
}

#[test]
fn an_error_without_a_redis_cause_changes_nothing() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let error = timeout_error();
    gate.record_failure(&error);
    let admission = gate
        .enter("async_batch_get_cache")
        .expect("the breaker is closed");
    gate.swallowed(&admission, &Broken);
    gate.exit(admission);
    assert_eq!(
        status(&gate).failure_count,
        0,
        "an unknown error leaves no marker, so the call still succeeds"
    );
}

#[test]
fn swallowed_counts_a_health_failure_too() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let admission = gate
        .enter("async_batch_get_cache")
        .expect("the breaker is closed");
    let error = connection_error();
    gate.swallowed(&admission, &error);
    let observed = status(&gate);
    assert_eq!(observed.failure_count, 1);
    assert_eq!(observed.hard_failure_count, 1);
}
