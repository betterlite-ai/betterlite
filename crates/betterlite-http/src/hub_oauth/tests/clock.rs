use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::sleep;

use crate::hub_oauth::clock::{HubClock as _, SystemHubClock};

#[test]
fn system_clock_reads_the_wall_clock() {
    let clock = SystemHubClock::new();
    let wall = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    assert!(clock.unix_seconds().abs_diff(wall) <= 5);
}

#[tokio::test]
async fn system_clock_counts_seconds_since_it_started() {
    let clock = SystemHubClock::new();
    sleep(Duration::from_millis(2100)).await;
    let elapsed = clock.monotonic_seconds();
    // scheduling may overshoot the sleep, but a clock that does not count reads 0 or 1 here
    assert!((2..60).contains(&elapsed), "elapsed {elapsed}");
}
