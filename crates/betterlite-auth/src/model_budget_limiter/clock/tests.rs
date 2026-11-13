#![cfg(test)]

use std::time::{Duration, Instant};

use chrono::Utc;

use super::{BudgetClock as _, SystemClock};

#[test]
fn the_system_clock_counts_the_process_uptime_from_its_start() {
    let started = Instant::now().checked_sub(Duration::from_secs(60)).unwrap();
    let clock = SystemClock { started };
    assert!(clock.uptime() >= Duration::from_secs(60));
    assert!(clock.uptime() < Duration::from_secs(120));
}

#[test]
fn the_system_clock_reports_the_current_time() {
    let clock = SystemClock {
        started: Instant::now(),
    };
    let before = Utc::now();
    let found = clock.now();
    let after = Utc::now();
    assert!(found >= before);
    assert!(found <= after);
}
