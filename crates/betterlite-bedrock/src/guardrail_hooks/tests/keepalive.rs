use std::time::Duration;

use crate::guardrail_hooks::keepalive::keepalive_ping_has_fired;

#[test]
fn a_ping_fires_once_the_elapsed_time_reaches_the_interval() {
    assert!(keepalive_ping_has_fired(
        Duration::from_secs(2),
        Some(1.0_f64)
    ));
    assert!(!keepalive_ping_has_fired(
        Duration::from_millis(500),
        Some(1.0_f64)
    ));
}

#[test]
fn a_negative_interval_never_fires() {
    assert!(!keepalive_ping_has_fired(
        Duration::from_secs(2),
        Some(-1.0_f64)
    ));
}

#[test]
fn a_zero_interval_never_fires() {
    assert!(!keepalive_ping_has_fired(Duration::ZERO, Some(0.0_f64)));
    assert!(!keepalive_ping_has_fired(
        Duration::from_secs(2),
        Some(0.0_f64)
    ));
}

#[test]
fn an_absent_interval_never_fires() {
    assert!(!keepalive_ping_has_fired(Duration::from_secs(2), None));
}
