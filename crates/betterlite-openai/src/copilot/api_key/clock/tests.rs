#![cfg(test)]

use std::time::{SystemTime, UNIX_EPOCH};

use super::Clock as _;
use crate::copilot::device_auth::sleeper::TokioSleeper;

fn unix_seconds() -> i64 {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    i64::try_from(elapsed.as_secs()).unwrap()
}

#[test]
fn the_tokio_sleeper_reads_whole_unix_seconds() {
    let before = unix_seconds();
    let now = TokioSleeper.now();
    let after = unix_seconds();

    assert!(
        (before..=after).contains(&now),
        "the clock read {now}, outside {before}..={after} unix seconds"
    );
}
