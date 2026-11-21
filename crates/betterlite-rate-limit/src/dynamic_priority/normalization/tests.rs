#![cfg(test)]

use std::collections::BTreeMap;
use std::ops::{Div as _, Sub as _};

use super::normalize_priority_weights;
use crate::dynamic_priority::tests::support::{capture, percent, policy, reserved};
use crate::provider_attribution::{ModelGroupLimits, PriorityKind};

fn weights(entries: &[(&str, f64)]) -> BTreeMap<String, f64> {
    entries
        .iter()
        .map(|(priority, weight)| ((*priority).to_owned(), *weight))
        .collect()
}

#[test]
fn no_reservation_normalizes_to_nothing() {
    assert_eq!(
        normalize_priority_weights(&policy(None), None),
        BTreeMap::new()
    );
}

#[test]
fn a_total_within_capacity_is_kept() {
    let configured = policy(Some(&[("key_a", percent(0.2)), ("key_b", percent(0.3))]));
    assert_eq!(
        normalize_priority_weights(&configured, None),
        weights(&[("key_a", 0.2), ("key_b", 0.3)])
    );
}

#[test]
fn a_total_of_exactly_one_is_not_renormalized() {
    let configured = policy(Some(&[("key_a", percent(0.5)), ("key_b", percent(0.5))]));
    let logs = capture(|| {
        assert_eq!(
            normalize_priority_weights(&configured, None),
            weights(&[("key_a", 0.5), ("key_b", 0.5)])
        );
    });
    assert!(
        !logs.contains("Normalized over-allocated priorities"),
        "logs: {logs}"
    );
}

#[test]
fn an_over_allocated_total_is_normalized_and_logged() {
    let configured = policy(Some(&[("key_a", percent(0.6)), ("key_b", percent(0.8))]));
    let logs = capture(|| {
        let normalized = normalize_priority_weights(&configured, None);
        assert!(normalized.values().sum::<f64>().sub(1.0_f64).abs() < 1e-12);
        assert!(normalized["key_a"].sub(0.6_f64.div(1.4_f64)).abs() < 1e-12);
        assert!(normalized["key_b"].sub(0.8_f64.div(1.4_f64)).abs() < 1e-12);
    });
    assert!(
        logs.contains("Normalized over-allocated priorities"),
        "logs: {logs}"
    );
}

#[test]
fn reserved_capacities_join_the_total() {
    let configured = policy(Some(&[
        ("key_a", reserved(PriorityKind::Rpm, 900.0)),
        ("key_b", percent(0.5)),
    ]));
    let limits = ModelGroupLimits {
        rpm: Some(1000),
        tpm: None,
    };
    let normalized = normalize_priority_weights(&configured, Some(&limits));
    assert!(normalized.values().sum::<f64>().sub(1.0_f64).abs() < 1e-12);
    assert!(normalized["key_b"].sub(0.5_f64.div(1.4_f64)).abs() < 1e-12);
}
