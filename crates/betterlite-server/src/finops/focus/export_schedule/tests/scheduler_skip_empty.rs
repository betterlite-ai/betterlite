#![cfg(test)]

use tracing::Level;

use crate::finops::focus::export_schedule::{RegisteredFocusLogger, init_focus_export_job};
use crate::finops::focus::support::{self, RecordingFactory, RecordingScheduler};

#[test]
fn an_empty_registry_skips_the_scheduler() {
    let (logs, _guard) = support::capture_logs();
    let scheduler = RecordingScheduler::<RecordingFactory>::new(false);
    let loggers: Vec<RegisteredFocusLogger<RecordingFactory>> = Vec::new();
    init_focus_export_job(&loggers, &scheduler).expect("skipping is not an error");
    assert!(scheduler.jobs().is_empty());
    assert!(logs.has_at(
        Level::DEBUG,
        "No Focus export logger registered; skipping scheduler"
    ));
}
