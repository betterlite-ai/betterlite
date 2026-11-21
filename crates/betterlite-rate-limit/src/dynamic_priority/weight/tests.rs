#![cfg(test)]

use super::priority_weight;
use crate::dynamic_priority::settings::{PriorityPolicy, PrioritySettings};
use crate::dynamic_priority::tests::support::{percent, policy, reserved};
use crate::provider_attribution::{ModelGroupLimits, PriorityKind};

#[test]
fn an_absent_reservation_reads_the_default_priority() {
    let configured = policy(None);
    assert_eq!(
        priority_weight(&configured, Some("prod"), None).to_bits(),
        0.25_f64.to_bits()
    );
    assert_eq!(
        priority_weight(&configured, None, None).to_bits(),
        0.25_f64.to_bits()
    );
}

#[test]
fn an_unreserved_priority_reads_the_default_priority() {
    let configured = policy(Some(&[("prod", percent(0.6))]));
    assert_eq!(
        priority_weight(&configured, Some("dev"), None).to_bits(),
        0.25_f64.to_bits()
    );
}

#[test]
fn a_configured_default_priority_answers_absent_priorities() {
    let configured = PriorityPolicy {
        settings: PrioritySettings {
            default_priority: 0.4,
            saturation_threshold: 0.50,
            saturation_check_cache_ttl: 60,
        },
        reservation: None,
    };
    assert_eq!(
        priority_weight(&configured, None, None).to_bits(),
        0.4_f64.to_bits()
    );
}

#[test]
fn a_percent_reservation_is_the_weight() {
    let configured = policy(Some(&[("prod", percent(0.6))]));
    assert_eq!(
        priority_weight(&configured, Some("prod"), None).to_bits(),
        0.6_f64.to_bits()
    );
}

#[test]
fn an_rpm_reservation_divides_the_model_capacity() {
    let configured = policy(Some(&[("prod", reserved(PriorityKind::Rpm, 600.0))]));
    let limits = ModelGroupLimits {
        rpm: Some(1000),
        tpm: None,
    };
    let weight = priority_weight(&configured, Some("prod"), Some(&limits));
    assert_eq!(weight.to_bits(), 0.6_f64.to_bits());
}

#[test]
fn an_rpm_reservation_without_capacity_reads_as_a_percent() {
    let configured = policy(Some(&[("prod", reserved(PriorityKind::Rpm, 600.0))]));
    let weight = priority_weight(&configured, Some("prod"), None);
    assert_eq!(weight.to_bits(), 600.0_f64.to_bits());
}

#[test]
fn a_tpm_reservation_divides_the_model_capacity() {
    let configured = policy(Some(&[("prod", reserved(PriorityKind::Tpm, 900.0))]));
    let limits = ModelGroupLimits {
        rpm: Some(1000),
        tpm: Some(9000),
    };
    let weight = priority_weight(&configured, Some("prod"), Some(&limits));
    assert_eq!(weight.to_bits(), 0.1_f64.to_bits());
}
