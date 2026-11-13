use std::time::Duration;

use super::super::constants::{DEFAULT_LANGUAGE, POLL_INTERVAL, READY_TIMEOUT};

#[test]
fn the_documented_defaults_keep_their_pinned_values() {
    assert_eq!(READY_TIMEOUT, Duration::from_secs(30));
    assert_eq!(POLL_INTERVAL, Duration::from_millis(200));
    assert_eq!(DEFAULT_LANGUAGE, "python");
}
