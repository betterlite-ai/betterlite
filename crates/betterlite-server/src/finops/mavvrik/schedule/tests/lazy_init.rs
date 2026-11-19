#![cfg(test)]

use std::sync::Arc;

use tracing::Level;

use crate::finops::mavvrik::schedule::init_mavvrik_background_job;
use crate::finops::mavvrik::support::{
    FakeFactory, FakeLazyInit, Recorder, RecordingScheduler, build, capture_logs, environment,
};

#[test]
fn a_configured_mavvrik_callback_is_built_lazily_and_scheduled() {
    let recorder = Recorder::default();
    let logger = Arc::new(build(&environment(&[]), &recorder, None));
    let lazy_init = FakeLazyInit::new(Some(Arc::clone(&logger)));
    let scheduler = RecordingScheduler::new(false);
    init_mavvrik_background_job(&[], true, &lazy_init, &scheduler)
        .expect("the lazy logger is scheduled");
    assert_eq!(lazy_init.integrations(), vec![String::from("mavvrik")]);
    let jobs = scheduler.jobs();
    assert_eq!(jobs.len(), 1);
    assert!(Arc::ptr_eq(&jobs[0].0, &logger));
}

#[test]
fn a_lazy_build_that_yields_no_logger_skips_the_scheduler() {
    let (logs, _guard) = capture_logs();
    let lazy_init = FakeLazyInit::<FakeFactory>::new(None);
    let scheduler = RecordingScheduler::new(false);
    init_mavvrik_background_job(&[], true, &lazy_init, &scheduler)
        .expect("nothing to schedule is not an error");
    assert!(scheduler.jobs().is_empty());
    assert_eq!(lazy_init.integrations(), vec![String::from("mavvrik")]);
    assert!(logs.has_at(
        Level::DEBUG,
        "No MavvrikFocusLogger registered; skipping scheduler"
    ));
}

#[test]
fn an_unconfigured_callback_never_reaches_lazy_init() {
    let lazy_init = FakeLazyInit::<FakeFactory>::new(None);
    let scheduler = RecordingScheduler::new(false);
    init_mavvrik_background_job(&[], false, &lazy_init, &scheduler)
        .expect("nothing to schedule is not an error");
    assert!(lazy_init.integrations().is_empty());
    assert!(scheduler.jobs().is_empty());
}
