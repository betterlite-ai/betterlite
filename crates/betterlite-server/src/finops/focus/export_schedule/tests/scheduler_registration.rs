#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::finops::focus::error::FocusInitError;
use crate::finops::focus::export_schedule::{
    FocusLoggerType, RegisteredFocusLogger, ScheduleTrigger, init_focus_export_job,
};
use crate::finops::focus::support::{self, Recorder, RecordingFactory, RecordingScheduler};

#[test]
fn a_focus_logger_registers_with_its_trigger() {
    let recorder = Recorder::new(false);
    let mut params = support::params();
    params.frequency = Some("daily".to_owned());
    params.cron_offset_minute = Some(90);
    let focus = Arc::new(support::build(
        &BTreeMap::new(),
        params,
        recorder.factory(),
        None,
    ));
    let loggers = vec![RegisteredFocusLogger {
        logger: Arc::clone(&focus),
        logger_type: FocusLoggerType::Focus,
    }];
    let scheduler = RecordingScheduler::<RecordingFactory>::new(false);
    init_focus_export_job(&loggers, &scheduler).expect("the job registers");
    let jobs = scheduler.jobs();
    let [(registered_logger, trigger)] = jobs.as_slice() else {
        panic!("exactly one job is registered");
    };
    assert!(Arc::ptr_eq(registered_logger, &focus));
    assert_eq!(
        *trigger,
        ScheduleTrigger::Cron {
            hour: Some(1),
            minute: 30,
            second: 0,
        }
    );
}

#[test]
fn a_rejecting_scheduler_surfaces_its_error() {
    let recorder = Recorder::new(false);
    let focus = Arc::new(support::build(
        &BTreeMap::new(),
        support::params(),
        recorder.factory(),
        None,
    ));
    let loggers = vec![RegisteredFocusLogger {
        logger: focus,
        logger_type: FocusLoggerType::Focus,
    }];
    let scheduler = RecordingScheduler::<RecordingFactory>::new(true);
    let error = init_focus_export_job(&loggers, &scheduler).expect_err("the scheduler rejects it");
    assert!(matches!(&error, FocusInitError::Scheduler(_)));
    assert_eq!(
        error.to_string(),
        "the FOCUS export job could not be scheduled"
    );
    assert!(scheduler.jobs().is_empty());
}

#[test]
fn an_unsupported_frequency_fails_before_the_scheduler() {
    let recorder = Recorder::new(false);
    let mut params = support::params();
    params.frequency = Some("weekly".to_owned());
    let focus = Arc::new(support::build(
        &BTreeMap::new(),
        params,
        recorder.factory(),
        None,
    ));
    let loggers = vec![RegisteredFocusLogger {
        logger: focus,
        logger_type: FocusLoggerType::Focus,
    }];
    let scheduler = RecordingScheduler::<RecordingFactory>::new(false);
    let error = init_focus_export_job(&loggers, &scheduler).expect_err("weekly has no trigger");
    assert!(matches!(&error, FocusInitError::Schedule(_)));
    assert_eq!(error.to_string(), "Unsupported frequency: weekly");
    assert!(scheduler.jobs().is_empty());
}
