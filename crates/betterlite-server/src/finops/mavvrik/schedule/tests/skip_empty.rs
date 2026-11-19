#![cfg(test)]

use std::sync::Arc;

use tracing::Level;

use crate::finops::mavvrik::schedule::{
    MavvrikLoggerType, RegisteredMavvrikLogger, init_mavvrik_background_job,
};
use crate::finops::mavvrik::support::{
    FakeFactory, FakeLazyInit, Recorder, RecordingScheduler, build, capture_logs, environment,
};

#[test]
fn no_logger_and_no_configured_name_skips_with_a_debug_line() {
    let (logs, _guard) = capture_logs();
    let lazy_init = FakeLazyInit::<FakeFactory>::new(None);
    let scheduler = RecordingScheduler::new(false);
    init_mavvrik_background_job(&[], false, &lazy_init, &scheduler)
        .expect("skipping is not an error");
    assert!(scheduler.jobs().is_empty());
    assert_eq!(lazy_init.integrations(), Vec::<String>::new());
    assert!(logs.has_at(
        Level::DEBUG,
        "No MavvrikFocusLogger registered; skipping scheduler"
    ));
}

#[test]
fn a_registered_subclass_alone_counts_as_no_logger() {
    let (logs, _guard) = capture_logs();
    let subclass = Arc::new(build(&environment(&[]), &Recorder::default(), None));
    let registered = vec![RegisteredMavvrikLogger {
        logger: subclass,
        logger_type: MavvrikLoggerType::Subclass,
    }];
    let lazy_init = FakeLazyInit::<FakeFactory>::new(None);
    let scheduler = RecordingScheduler::new(false);
    init_mavvrik_background_job(&registered, false, &lazy_init, &scheduler)
        .expect("skipping is not an error");
    assert!(scheduler.jobs().is_empty());
    assert!(logs.has_at(
        Level::DEBUG,
        "No MavvrikFocusLogger registered; skipping scheduler"
    ));
}
