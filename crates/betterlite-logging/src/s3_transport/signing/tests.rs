#![cfg(test)]

use std::time::{Duration, SystemTime};

use crate::s3_transport::signing::{Clock as _, SystemClock};

#[test]
fn the_system_clock_reads_the_current_time() {
    let before = SystemTime::now()
        .checked_sub(Duration::from_secs(1))
        .expect("the clock fits");
    let now = SystemClock.now();
    assert!(now >= before);
}
