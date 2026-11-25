#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate as _;

use super::super::SWALLOWED_CALLS_KEPT;
use super::support::{
    FakeClock, command_error, gate_with, settings, status, swallow_calls, timeout_error,
};

#[test]
fn a_swallowed_failure_marks_only_its_own_call() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(2), &clock);
    let error = timeout_error();
    gate.record_failure(&error);
    let failing = gate
        .enter("async_batch_get_cache")
        .expect("the breaker is closed");
    let healthy = gate
        .enter("async_batch_get_cache")
        .expect("the breaker is closed");
    gate.swallowed(&failing, &error);
    assert_eq!(status(&gate).failure_count, 2);
    gate.exit(healthy);
    assert_eq!(
        status(&gate).failure_count,
        0,
        "a concurrent call that saw no failure still records its success"
    );
    let later = timeout_error();
    gate.record_failure(&later);
    assert_eq!(status(&gate).failure_count, 1);
    gate.exit(failing);
    assert_eq!(
        status(&gate).failure_count,
        1,
        "the call that swallowed a failure records none"
    );
}

#[test]
fn an_outer_call_still_records_its_success_when_an_inner_call_swallowed() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(2), &clock);
    let error = timeout_error();
    let outer = gate
        .enter("batch_cache_write")
        .expect("the breaker is closed");
    let inner = gate
        .enter("async_set_cache_pipeline")
        .expect("the breaker is closed");
    gate.swallowed(&inner, &error);
    gate.exit(inner);
    assert_eq!(status(&gate).failure_count, 1);
    gate.exit(outer);
    assert_eq!(
        status(&gate).failure_count,
        0,
        "the outer call records its success even though the inner call swallowed a failure"
    );
}

#[test]
fn a_swallowed_command_error_leaves_no_marker() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(2), &clock);
    let error = timeout_error();
    gate.record_failure(&error);
    let admission = gate
        .enter("async_batch_get_cache")
        .expect("the breaker is closed");
    let command = command_error();
    gate.swallowed(&admission, &command);
    assert_eq!(
        status(&gate).failure_count,
        1,
        "a command error is no health failure"
    );
    gate.exit(admission);
    assert_eq!(
        status(&gate).failure_count,
        0,
        "the call still records its success"
    );
}

#[test]
fn the_swallow_set_stays_at_its_cap() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(u32::MAX), &clock);
    let error = timeout_error();
    let admissions = swallow_calls(&gate, &error, SWALLOWED_CALLS_KEPT + 1);
    gate.exit(admissions[SWALLOWED_CALLS_KEPT]);
    assert_ne!(
        status(&gate).failure_count,
        0,
        "the newest swallowed call keeps its marker"
    );
    gate.exit(admissions[0]);
    assert_eq!(
        status(&gate).failure_count,
        0,
        "the oldest marker was dropped, so its exit records a success"
    );
}
