#![cfg(test)]

use chrono::Utc;

use crate::langsmith_logger::clock::{Clock as _, SystemClock};

#[test]
fn the_system_clock_reports_the_current_time() {
    let before = Utc::now();
    let reported = SystemClock.now();
    let after = Utc::now();
    assert!(reported >= before);
    assert!(reported <= after);
}
