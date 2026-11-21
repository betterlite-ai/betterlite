#![cfg(test)]

use super::saturation_check_cache_ttl;
use crate::dynamic_priority::settings::{PriorityPolicy, PrioritySettings};

#[test]
fn the_default_ttl_is_sixty_seconds() {
    assert_eq!(saturation_check_cache_ttl(&PriorityPolicy::default()), 60);
}

#[test]
fn a_configured_ttl_is_read() {
    let configured = PriorityPolicy {
        settings: PrioritySettings {
            default_priority: 0.25,
            saturation_threshold: 0.50,
            saturation_check_cache_ttl: 120,
        },
        reservation: None,
    };
    assert_eq!(saturation_check_cache_ttl(&configured), 120);
}
