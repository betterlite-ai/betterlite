#![cfg(test)]

use betterlite_config::sections::messages_native_stream::Settings;
use rstest::rstest;

use crate::api::messages_native_stream::RelayLimits;

#[test]
fn the_settings_become_relay_limits() {
    let defaults =
        RelayLimits::from_settings(&Settings::default()).expect("the defaults are valid");
    assert_eq!(
        (defaults.queue_maxsize, defaults.max_detached_stream_drains),
        (1024, 100)
    );
    let explicit = RelayLimits::from_settings(&Settings {
        queue_maxsize: 2,
        max_detached_stream_drains: 1,
    })
    .expect("positive limits are valid");
    assert_eq!(
        (explicit.queue_maxsize, explicit.max_detached_stream_drains),
        (2, 1)
    );
}

#[rstest]
#[case::zero(0)]
#[case::negative(-1)]
fn relay_limits_reject_a_non_positive_queue(#[case] queue_maxsize: i64) {
    let error = RelayLimits::from_settings(&Settings {
        queue_maxsize,
        max_detached_stream_drains: 100,
    })
    .expect_err("a non-positive queue fails startup before any listener binds");
    assert_eq!(
        error.to_string(),
        "ANTHROPIC_MESSAGES_STREAM_RELAY_QUEUE_MAXSIZE must be greater than 0"
    );
}

#[rstest]
#[case::zero(0)]
#[case::negative(-1)]
fn a_non_positive_drain_cap_disables_draining(#[case] max_detached_stream_drains: i64) {
    let limits = RelayLimits::from_settings(&Settings {
        queue_maxsize: 1024,
        max_detached_stream_drains,
    })
    .expect("the queue is positive");
    assert_eq!(limits.max_detached_stream_drains, 0);
}
