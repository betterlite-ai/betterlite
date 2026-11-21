#![cfg(test)]

use std::collections::BTreeMap;

use super::{PriorityAllocation, priority_allocation};
use crate::dynamic_priority::settings::PriorityPolicy;
use crate::dynamic_priority::tests::support::{percent, policy};

fn normalized(entries: &[(&str, f64)]) -> BTreeMap<String, f64> {
    entries
        .iter()
        .map(|(priority, weight)| ((*priority).to_owned(), *weight))
        .collect()
}

fn allocation(
    configured: &PriorityPolicy,
    priority: Option<&str>,
    weights: &BTreeMap<String, f64>,
) -> PriorityAllocation {
    priority_allocation(configured, "gpt-4", priority, weights, None)
}

#[test]
fn an_explicit_priority_gets_its_own_pool() {
    let configured = policy(Some(&[("prod", percent(0.5)), ("dev", percent(0.25))]));
    let weights = normalized(&[("prod", 0.5), ("dev", 0.25)]);
    assert_eq!(
        allocation(&configured, Some("prod"), &weights),
        PriorityAllocation {
            weight: 0.5,
            key: "gpt-4:prod".to_owned(),
        }
    );
}

#[test]
fn a_priority_missing_from_the_normalized_weights_is_converted_again() {
    let configured = policy(Some(&[("prod", percent(0.5))]));
    assert_eq!(
        allocation(&configured, Some("prod"), &BTreeMap::new()),
        PriorityAllocation {
            weight: 0.5,
            key: "gpt-4:prod".to_owned(),
        }
    );
}

#[test]
fn no_priority_shares_the_default_pool() {
    let configured = policy(Some(&[("prod", percent(0.5))]));
    assert_eq!(
        allocation(&configured, None, &normalized(&[("prod", 0.5)])),
        PriorityAllocation {
            weight: 0.25,
            key: "gpt-4:default_pool".to_owned(),
        }
    );
}

#[test]
fn an_unreserved_priority_shares_the_default_pool() {
    let configured = policy(Some(&[("prod", percent(0.5))]));
    assert_eq!(
        allocation(&configured, Some("dev"), &normalized(&[("prod", 0.5)])),
        PriorityAllocation {
            weight: 0.25,
            key: "gpt-4:default_pool".to_owned(),
        }
    );
}

#[test]
fn an_absent_reservation_shares_the_default_pool() {
    let configured = policy(None);
    assert_eq!(
        allocation(&configured, Some("prod"), &BTreeMap::new()),
        PriorityAllocation {
            weight: 0.25,
            key: "gpt-4:default_pool".to_owned(),
        }
    );
}
