#![cfg(test)]

use tracing::Level;

use crate::finops::vantage::export_schedule::init_vantage_background_job;
use crate::finops::vantage::store::VantageLoggers;
use crate::finops::vantage::support::{self, RecordingScheduler};

#[test]
fn an_empty_store_skips_the_scheduler() {
    let (logs, _guard) = support::capture_logs();
    let store = VantageLoggers::new();
    let scheduler = RecordingScheduler::new(false);
    init_vantage_background_job(&store, &scheduler).expect("an empty store is not an error");
    assert!(scheduler.jobs().is_empty());
    assert!(logs.has_at(
        Level::DEBUG,
        "No Vantage logger registered; skipping scheduler"
    ));
}
