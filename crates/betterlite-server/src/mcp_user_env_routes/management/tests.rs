#![cfg(test)]

use chrono::{TimeDelta, Utc};

use crate::failure_hook::ports::Clock as _;
use crate::management_effects::wrapper::SystemClock;

#[test]
fn the_management_clock_reads_the_wall_clock() {
    let now = SystemClock.now();
    let five_minutes_ago = Utc::now()
        .fixed_offset()
        .checked_sub_signed(TimeDelta::minutes(5))
        .unwrap();
    assert!(now > five_minutes_ago);
}
