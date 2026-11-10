use std::time::Duration;

use tokio::time::Instant;

use crate::config_sync::clock::{ConfigSyncClock as _, MonotonicClock};
use crate::config_sync::jitter::{JitterSource as _, SystemJitter};

#[tokio::test]
async fn the_monotonic_clock_advances_with_its_pauses() {
    let clock = MonotonicClock {
        origin: Instant::now(),
    };
    let before = clock.now();
    clock.pause(Duration::from_millis(5)).await;
    let after = clock.now();
    assert!(after.saturating_sub(before) >= Duration::from_millis(5));
}

#[test]
fn the_system_jitter_stays_inside_the_limit_and_can_vary() {
    let jitter = SystemJitter::try_new().expect("a seeded jitter source");
    let limit_nanos = 10_000_000_u64;
    let inside = (0_i32..100_i32).all(|_| jitter.uniform_nanos(limit_nanos) < limit_nanos);
    let varied = (0_i32..100_i32).any(|_| jitter.uniform_nanos(limit_nanos) > 0);
    assert!(inside);
    assert!(varied);
    assert_eq!(jitter.uniform_nanos(0), 0);
}
