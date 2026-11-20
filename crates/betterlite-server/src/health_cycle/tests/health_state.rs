#![cfg(test)]

use std::sync::atomic::Ordering;

use super::state::build_deployment_health_states;
use super::support::{counting_clock, fixed_clock, number, object, text};

#[test]
fn a_healthy_row_is_true_with_an_empty_reason() {
    let healthy = [object(vec![
        ("model_id", text("a")),
        ("model", text("openai/a")),
    ])];
    let states = build_deployment_health_states(&healthy, &[], &fixed_clock(1_700_000_000.5));
    assert_eq!(states.len(), 1);
    let state = states
        .get("a")
        .expect("the healthy row is filed under its id");
    assert!(state.is_healthy);
    assert_eq!(state.timestamp.to_bits(), 1_700_000_000.5_f64.to_bits());
    assert_eq!(state.reason, "");
}

#[test]
fn an_unhealthy_row_overwrites_a_healthy_row_of_the_same_id() {
    let healthy = [object(vec![("model_id", text("a"))])];
    let unhealthy = [object(vec![("model_id", text("a"))])];
    let states = build_deployment_health_states(&healthy, &unhealthy, &fixed_clock(10.0));
    let state = states.get("a").expect("the last verdict wins");
    assert!(!state.is_healthy);
    assert_eq!(state.reason, "background_health_check_failed");
}

#[test]
fn each_row_keeps_its_own_verdict() {
    let healthy = [
        object(vec![("model_id", text("a"))]),
        object(vec![("model_id", text("b"))]),
    ];
    let unhealthy = [object(vec![("model_id", text("b"))])];
    let states = build_deployment_health_states(&healthy, &unhealthy, &fixed_clock(10.0));
    assert_eq!(states.len(), 2);
    assert!(states.get("a").expect("a is healthy").is_healthy);
    assert!(!states.get("b").expect("b is unhealthy").is_healthy);
}

#[test]
fn rows_without_a_usable_id_are_skipped() {
    let healthy = [
        object(vec![("model", text("openai/a"))]),
        object(vec![("model_id", text(""))]),
        object(vec![("model_id", number(7))]),
    ];
    let unhealthy = [
        object(vec![("model", text("openai/b"))]),
        object(vec![("model_id", text(""))]),
        object(vec![("model_id", number(7))]),
    ];
    let states = build_deployment_health_states(&healthy, &unhealthy, &fixed_clock(10.0));
    assert!(states.is_empty());
}

#[test]
fn the_cycle_timestamp_is_one_clock_read() {
    let (clock, calls) = counting_clock(42.0);
    let healthy = [object(vec![("model_id", text("a"))])];
    let unhealthy = [object(vec![("model_id", text("b"))])];
    let states = build_deployment_health_states(&healthy, &unhealthy, &clock);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let timestamps: Vec<u64> = states
        .values()
        .map(|state| state.timestamp.to_bits())
        .collect();
    assert_eq!(timestamps, vec![42.0_f64.to_bits(), 42.0_f64.to_bits()]);
}
