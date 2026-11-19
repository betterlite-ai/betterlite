use chrono::Utc;

use crate::auth_invalidation::clock::{InvalidationClock, SystemClock};

#[test]
fn system_clock_reads_the_wall_clock() {
    let started = Utc::now();
    let now = SystemClock.now();
    let finished = Utc::now();
    assert!(
        now >= started,
        "the system clock is not behind the reading that started it"
    );
    assert!(
        now <= finished,
        "the system clock is not ahead of the reading that ended it"
    );
}
