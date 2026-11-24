use std::thread;
use std::time::Duration;

use crate::api::responses_poll_stream::clock::{PollClock as _, SystemClock};

#[test]
fn system_clock_counts_elapsed_millis() {
    let clock = SystemClock::started();
    thread::park_timeout(Duration::from_millis(5));
    assert!(clock.now_millis() >= 5);
}
