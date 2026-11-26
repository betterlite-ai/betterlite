use std::time::{Duration, Instant};

use crate::jwt_keys::clock::{Clock, SystemClock};

#[tokio::test]
async fn system_clock_reads_the_wall_clock_and_sleeps() {
    let clock = SystemClock;
    let now = clock.now();
    assert!(now > 1_700_000_000.0_f64, "{now} is an epoch in seconds");

    let started = Instant::now();
    clock.sleep(Duration::from_millis(20)).await;
    assert!(
        started.elapsed() >= Duration::from_millis(10),
        "the retry backoff waits instead of returning at once"
    );
}
