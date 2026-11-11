use std::time::{SystemTime, UNIX_EPOCH};

use crate::datadog_logging::clock::{Clock as _, SystemClock};

#[test]
fn clock_reports_the_current_epoch_second() {
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("now is after the unix epoch")
        .as_secs();
    let now = SystemClock.now_unix().as_secs();
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("now is after the unix epoch")
        .as_secs();
    assert!((before..=after).contains(&now));
}
