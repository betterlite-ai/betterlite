use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::{
    FIXED_NOW_SECONDS, Scripted, guardrail_with_budget, input_scan, message, passing, run_at,
    too_large,
};

static CLOCK_CALLS: AtomicU64 = AtomicU64::new(0);

// every clock read lands one second after the last, so a send can only carry its own moment
fn stepping_clock() -> SystemTime {
    let step = CLOCK_CALLS.fetch_add(1, Ordering::SeqCst);
    UNIX_EPOCH
        .checked_add(Duration::from_secs(FIXED_NOW_SECONDS.saturating_add(step)))
        .unwrap_or(UNIX_EPOCH)
}

#[tokio::test]
async fn every_send_is_signed_with_the_clock_at_that_moment() {
    let messages = vec![
        message("user", "aaaa"),
        message("user", "bbbb"),
        message("user", "cccc"),
    ];
    let transport = Scripted::new(vec![too_large(), passing(5), passing(7)]);
    let mut data = BTreeMap::new();
    let outcome = run_at(
        &guardrail_with_budget(8),
        &mut data,
        input_scan(&messages),
        &transport,
        stepping_clock,
    )
    .await;
    drop(outcome.expect("both batches pass"));
    let stamps: Vec<String> = transport
        .header_values("x-amz-date")
        .into_iter()
        .flatten()
        .collect();
    assert_eq!(stamps.len(), 3);
    assert_ne!(stamps[0], stamps[1]);
    assert_ne!(stamps[1], stamps[2]);
    assert_ne!(stamps[0], stamps[2]);
}
