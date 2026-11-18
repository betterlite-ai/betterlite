use chrono::Utc;

use crate::model_deprecation::clock::{SystemClock, UtcClock};

#[test]
fn system_clock_reads_the_wall_clock() {
    let before = Utc::now();
    let now = SystemClock.now();
    let after = Utc::now();
    assert!(before <= now && now <= after);
}
