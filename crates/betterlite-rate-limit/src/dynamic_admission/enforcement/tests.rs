#![cfg(test)]

use super::plan_enforcement;
use crate::atomic_counter::payload::IncrementAmounts;
use crate::dynamic_admission::tests::support::MODEL;
use crate::dynamic_priority::descriptors::{PRIORITY_DESCRIPTOR_KEY, SATURATION_DESCRIPTOR_KEY};
use crate::limiter_records::RateLimitDescriptor;

const THRESHOLD: f64 = 0.5;

fn descriptor(key: &str, value: &str) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: key.to_owned(),
        value: value.to_owned(),
        rate_limit: None,
    }
}

fn model_wide() -> RateLimitDescriptor {
    descriptor(SATURATION_DESCRIPTOR_KEY, MODEL)
}

fn priority_pool(priority: &str) -> RateLimitDescriptor {
    descriptor(PRIORITY_DESCRIPTOR_KEY, priority)
}

#[test]
fn enforcement_below_threshold_sends_only_the_model_wide_descriptor() {
    let plan = plan_enforcement(
        model_wide(),
        vec![priority_pool("gpt-4:default")],
        0.3,
        THRESHOLD,
    );
    assert_eq!(plan.enforced, vec![model_wide()]);
    assert_eq!(plan.tracked_only, vec![priority_pool("gpt-4:default")]);
}

#[test]
fn enforcement_at_the_threshold_enforces_the_priority_descriptors() {
    let plan = plan_enforcement(
        model_wide(),
        vec![priority_pool("gpt-4:default")],
        THRESHOLD,
        THRESHOLD,
    );
    assert_eq!(
        plan.enforced,
        vec![model_wide(), priority_pool("gpt-4:default")]
    );
    assert!(plan.tracked_only.is_empty());
}

#[test]
fn enforcement_above_the_threshold_enforces_the_priority_descriptors() {
    let plan = plan_enforcement(
        model_wide(),
        vec![priority_pool("gpt-4:default")],
        0.6,
        THRESHOLD,
    );
    assert_eq!(
        plan.enforced,
        vec![model_wide(), priority_pool("gpt-4:default")]
    );
    assert!(plan.tracked_only.is_empty());
}

#[test]
fn enforcement_without_priority_descriptors_tracks_nothing() {
    let plan = plan_enforcement(model_wide(), Vec::new(), 0.9, THRESHOLD);
    assert_eq!(plan.enforced, vec![model_wide()]);
    assert!(plan.tracked_only.is_empty());
}

#[test]
fn enforcement_keeps_the_model_wide_descriptor_first_and_the_pool_order() {
    let plan = plan_enforcement(
        model_wide(),
        vec![priority_pool("gpt-4:first"), priority_pool("gpt-4:second")],
        0.8,
        THRESHOLD,
    );
    assert_eq!(plan.enforced[0], model_wide());
    assert_eq!(plan.enforced[1], priority_pool("gpt-4:first"));
    assert_eq!(plan.enforced[2], priority_pool("gpt-4:second"));
}

#[test]
fn enforcement_gives_every_enforced_descriptor_the_same_increment() {
    let plan = plan_enforcement(
        model_wide(),
        vec![priority_pool("gpt-4:default")],
        0.9,
        THRESHOLD,
    );
    let increment = IncrementAmounts {
        requests: 1,
        tokens: 0,
    };
    assert_eq!(plan.increments.len(), plan.enforced.len());
    assert_eq!(plan.increments, vec![increment, increment]);
}

#[test]
fn enforcement_counts_the_model_wide_increment_alone_below_the_threshold() {
    let plan = plan_enforcement(
        model_wide(),
        vec![priority_pool("gpt-4:default")],
        0.1,
        THRESHOLD,
    );
    assert_eq!(
        plan.increments,
        vec![IncrementAmounts {
            requests: 1,
            tokens: 0
        }]
    );
}

#[test]
fn enforcement_without_priority_descriptors_increments_only_the_model_wide() {
    let plan = plan_enforcement(model_wide(), Vec::new(), 0.9, THRESHOLD);
    assert_eq!(
        plan.increments,
        vec![IncrementAmounts {
            requests: 1,
            tokens: 0
        }]
    );
}
