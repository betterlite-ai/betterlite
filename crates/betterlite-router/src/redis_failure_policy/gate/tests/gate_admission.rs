#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate as _;

use super::support::{
    FakeClock, command_error, connection_error, gate_with, settings, status, timeout_error,
};
use crate::redis_failure_policy::State;

#[test]
fn enter_admits_while_the_breaker_is_closed() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(3), &clock);
    let admission = gate
        .enter("async_get_cache")
        .expect("a closed breaker admits the call");
    gate.exit(admission);
    assert_eq!(status(&gate).state, State::Closed);
    assert_eq!(status(&gate).failure_count, 0);
}

#[test]
fn enter_returns_the_method_name_when_the_breaker_is_open() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(1), &clock);
    let error = connection_error();
    gate.record_failure(&error);
    let refused = gate
        .enter("async_get_cache")
        .expect_err("the open breaker refuses the call");
    assert_eq!(refused.name, "async_get_cache");
}

#[test]
fn enter_admits_a_probe_after_the_recovery_timeout() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(1), &clock);
    let error = connection_error();
    gate.record_failure(&error);
    assert!(
        gate.enter("async_get_cache").is_err(),
        "the breaker just opened"
    );
    clock.advance(10_000);
    assert!(
        gate.enter("async_get_cache").is_err(),
        "the recovery timeout has not passed yet"
    );
    clock.advance(1);
    let probe = gate
        .enter("async_get_cache")
        .expect("the probe is admitted once the timeout passes");
    assert_eq!(status(&gate).state, State::HalfOpen);
    gate.exit(probe);
    assert_eq!(status(&gate).state, State::Closed);
}

#[test]
fn a_non_health_probe_error_frees_the_slot() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(1), &clock);
    gate.record_failure(&connection_error());
    clock.advance(10_001);
    let _probe = gate
        .enter("async_get_cache")
        .expect("the designated probe is admitted");
    gate.record_failure(&command_error());
    let now = (gate.clock)();
    gate.policy
        .lock()
        .expect("the policy lock is not poisoned")
        .admit(now, "async_get_cache")
        .expect("the non-health error freed the probe slot");
}

#[test]
fn a_non_health_swallowed_error_frees_the_slot() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(1), &clock);
    gate.record_failure(&connection_error());
    clock.advance(10_001);
    let probe = gate
        .enter("async_get_cache")
        .expect("the designated probe is admitted");
    gate.swallowed(&probe, &command_error());
    let now = (gate.clock)();
    gate.policy
        .lock()
        .expect("the policy lock is not poisoned")
        .admit(now, "async_get_cache")
        .expect("the non-health error freed the probe slot");
}

#[test]
fn a_cancelled_probe_frees_the_slot() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(1), &clock);
    gate.record_failure(&connection_error());
    clock.advance(10_001);
    // the probe's future is dropped before it can exit, the way a cancelled request ends
    let _cancelled = gate
        .enter("async_get_cache")
        .expect("the designated probe is admitted");
    gate.enter("async_get_cache")
        .expect("the abandoned probe slot admits the next call");
}

#[test]
fn exit_records_a_success() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(2), &clock);
    let error = timeout_error();
    gate.record_failure(&error);
    gate.record_failure(&error);
    assert_eq!(status(&gate).failure_count, 2);
    let first = gate
        .enter("async_get_cache")
        .expect("the breaker is closed");
    let second = gate
        .enter("async_get_cache")
        .expect("the breaker is closed");
    gate.exit(second);
    assert_eq!(
        status(&gate).failure_count,
        0,
        "a call that exits without a failure records a success"
    );
    gate.exit(first);
    assert_eq!(status(&gate).failure_count, 0);
}

#[test]
fn exit_records_no_success_when_the_generation_moved() {
    let clock = Arc::new(FakeClock::new());
    let gate = gate_with(settings(2), &clock);
    let error = connection_error();
    let early = gate
        .enter("async_get_cache")
        .expect("the breaker is closed");
    gate.record_failure(&error);
    gate.record_failure(&error);
    assert_eq!(status(&gate).state, State::Open);
    clock.advance(10_001);
    let probe = gate
        .enter("async_get_cache")
        .expect("the probe is admitted");
    gate.exit(probe);
    assert_eq!(
        status(&gate).state,
        State::Closed,
        "the probe's success closed the breaker"
    );
    let later = timeout_error();
    gate.record_failure(&later);
    assert_eq!(status(&gate).failure_count, 1);
    gate.exit(early);
    assert_eq!(
        status(&gate).failure_count,
        1,
        "a call admitted before the breaker moved records no success"
    );
}
