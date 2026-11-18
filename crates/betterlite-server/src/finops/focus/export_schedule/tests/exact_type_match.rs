#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use tracing::Level;

use crate::finops::focus::export_schedule::{
    FocusLoggerType, RegisteredFocusLogger, init_focus_export_job,
};
use crate::finops::focus::support::{self, Recorder, RecordingFactory, RecordingScheduler};

#[test]
fn a_vantage_logger_listed_first_is_not_the_job() {
    let recorder = Recorder::new(false);
    let vantage = Arc::new(support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    ));
    let focus = Arc::new(support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    ));
    let loggers = vec![
        RegisteredFocusLogger {
            logger: Arc::clone(&vantage),
            logger_type: FocusLoggerType::Vantage,
        },
        RegisteredFocusLogger {
            logger: Arc::clone(&focus),
            logger_type: FocusLoggerType::Focus,
        },
    ];
    let scheduler = RecordingScheduler::<RecordingFactory>::new(false);
    init_focus_export_job(&loggers, &scheduler).expect("the job registers");
    let jobs = scheduler.jobs();
    let [(registered_logger, _trigger)] = jobs.as_slice() else {
        panic!("exactly one job is registered");
    };
    assert!(Arc::ptr_eq(registered_logger, &focus));
    assert!(!Arc::ptr_eq(registered_logger, &vantage));
}

#[test]
fn a_vantage_logger_alone_skips_the_scheduler() {
    let (logs, _guard) = support::capture_logs();
    let recorder = Recorder::new(false);
    let vantage = Arc::new(support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    ));
    let loggers = vec![RegisteredFocusLogger {
        logger: vantage,
        logger_type: FocusLoggerType::Vantage,
    }];
    let scheduler = RecordingScheduler::<RecordingFactory>::new(false);
    init_focus_export_job(&loggers, &scheduler).expect("skipping is not an error");
    assert!(scheduler.jobs().is_empty());
    assert!(logs.has_at(
        Level::DEBUG,
        "No Focus export logger registered; skipping scheduler"
    ));
}

#[test]
fn the_first_focus_logger_registers() {
    let recorder = Recorder::new(false);
    let first = Arc::new(support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    ));
    let second = Arc::new(support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    ));
    let loggers = vec![
        RegisteredFocusLogger {
            logger: Arc::clone(&first),
            logger_type: FocusLoggerType::Focus,
        },
        RegisteredFocusLogger {
            logger: Arc::clone(&second),
            logger_type: FocusLoggerType::Focus,
        },
    ];
    let scheduler = RecordingScheduler::<RecordingFactory>::new(false);
    init_focus_export_job(&loggers, &scheduler).expect("the job registers");
    let jobs = scheduler.jobs();
    let [(registered_logger, _trigger)] = jobs.as_slice() else {
        panic!("exactly one job is registered");
    };
    assert!(Arc::ptr_eq(registered_logger, &first));
    assert!(!Arc::ptr_eq(registered_logger, &second));
}
