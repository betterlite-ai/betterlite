#![cfg(test)]

use prometheus_client::encoding::text::encode;
use prometheus_client::registry::Registry;

use super::BreakerMetrics;

#[test]
fn registers_the_three_named_metrics() {
    let mut registry = Registry::default();
    let metrics = BreakerMetrics::new(Some(&mut registry));
    metrics.record_state_change(None, "closed");
    metrics.record_transition("closed");
    metrics.record_failure("timeout");

    let mut buffer = String::new();
    encode(&mut buffer, &registry).unwrap();
    assert!(buffer.contains("litellm_redis_circuit_breaker_state"));
    assert!(buffer.contains("litellm_redis_circuit_breaker_transitions"));
    assert!(buffer.contains("litellm_redis_circuit_breaker_failures"));
}

#[test]
fn records_an_initial_state_without_an_old_state() {
    let mut registry = Registry::default();
    let metrics = BreakerMetrics::new(Some(&mut registry));
    metrics.record_state_change(None, "closed");

    let mut buffer = String::new();
    encode(&mut buffer, &registry).unwrap();
    assert!(buffer.contains("litellm_redis_circuit_breaker_state{state=\"closed\"} 1"));
}

#[test]
fn moving_state_decrements_the_old_state_and_increments_the_new_one() {
    let mut registry = Registry::default();
    let metrics = BreakerMetrics::new(Some(&mut registry));
    metrics.record_state_change(None, "closed");
    metrics.record_state_change(Some("closed"), "open");

    let mut buffer = String::new();
    encode(&mut buffer, &registry).unwrap();
    assert!(buffer.contains("litellm_redis_circuit_breaker_state{state=\"closed\"} 0"));
    assert!(buffer.contains("litellm_redis_circuit_breaker_state{state=\"open\"} 1"));
}

#[test]
fn counts_transitions_per_state() {
    let mut registry = Registry::default();
    let metrics = BreakerMetrics::new(Some(&mut registry));
    metrics.record_transition("open");
    metrics.record_transition("open");

    let mut buffer = String::new();
    encode(&mut buffer, &registry).unwrap();
    assert!(buffer.contains("litellm_redis_circuit_breaker_transitions_total{state=\"open\"} 2"));
}

#[test]
fn counts_failures_per_class() {
    let mut registry = Registry::default();
    let metrics = BreakerMetrics::new(Some(&mut registry));
    metrics.record_failure("timeout");
    metrics.record_failure("timeout");
    metrics.record_failure("connection_error");

    let mut buffer = String::new();
    encode(&mut buffer, &registry).unwrap();
    assert!(
        buffer
            .contains("litellm_redis_circuit_breaker_failures_total{failure_class=\"timeout\"} 2")
    );
    assert!(buffer.contains(
        "litellm_redis_circuit_breaker_failures_total{failure_class=\"connection_error\"} 1"
    ));
}

#[test]
fn no_ops_every_recording_call_without_a_registry() {
    let metrics = BreakerMetrics::new(None);
    metrics.record_state_change(None, "closed");
    metrics.record_state_change(Some("closed"), "open");
    metrics.record_transition("open");
    metrics.record_failure("timeout");
}
