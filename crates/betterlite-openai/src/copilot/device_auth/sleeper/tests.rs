#![cfg(test)]

use std::time::{Duration, Instant};

use super::{Sleeper as _, TokioSleeper};

#[tokio::test]
async fn tokio_sleeper_waits_for_the_requested_duration() {
    let mut sleeper = TokioSleeper;
    let started = Instant::now();

    sleeper.sleep(Duration::from_millis(20)).await;

    assert!(started.elapsed() >= Duration::from_millis(10));
}
