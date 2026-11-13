#![cfg(test)]

use std::time::{Duration, SystemTime};

use super::SystemClock;

use crate::sqs_logging::clock::Clock as _;

#[test]
fn system_clock_reads_the_wall_clock() {
    let now = SystemTime::now();
    let reported = SystemClock.now();
    let difference = now
        .duration_since(reported)
        .unwrap_or_else(|error| error.duration());
    assert!(difference <= Duration::from_secs(60));
}
