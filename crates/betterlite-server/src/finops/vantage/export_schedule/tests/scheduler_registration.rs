#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::finops::focus::export_schedule::{FocusPodLock, ScheduleError, ScheduleTrigger};
use crate::finops::vantage::error::VantageInitError;
use crate::finops::vantage::export::VantageExportLogger;
use crate::finops::vantage::export_schedule::init_vantage_background_job;
use crate::finops::vantage::store::{VantageCallbackList, VantageLoggers};
use crate::finops::vantage::support::{self, Recorder, RecordingScheduler};

#[test]
fn the_interval_frequency_registers_an_interval_job() {
    let recorder = Recorder::new(false);
    let mut params = support::params();
    params.frequency = Some("interval".to_owned());
    params.interval_seconds = Some(90);
    let logger = support::build(&BTreeMap::new(), params, &recorder, None);
    let registered: Arc<dyn VantageExportLogger> = Arc::new(logger);
    let store = VantageLoggers::new();
    store.register(Arc::clone(&registered), VantageCallbackList::Callbacks);
    let scheduler = RecordingScheduler::new(false);
    init_vantage_background_job(&store, &scheduler).expect("the job registers");
    let jobs = scheduler.jobs();
    assert_eq!(jobs.len(), 1);
    assert!(Arc::ptr_eq(&jobs[0].0, &registered));
    assert_eq!(jobs[0].1, ScheduleTrigger::Interval { seconds: 90 });
}

#[test]
fn the_hourly_default_registers_the_cron_job_at_the_default_offset() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        &recorder,
        None::<Arc<dyn FocusPodLock>>,
    );
    let registered: Arc<dyn VantageExportLogger> = Arc::new(logger);
    let store = VantageLoggers::new();
    store.register(Arc::clone(&registered), VantageCallbackList::Callbacks);
    let scheduler = RecordingScheduler::new(false);
    init_vantage_background_job(&store, &scheduler).expect("the job registers");
    let jobs = scheduler.jobs();
    assert_eq!(jobs.len(), 1);
    assert!(Arc::ptr_eq(&jobs[0].0, &registered));
    assert_eq!(
        jobs[0].1,
        ScheduleTrigger::Cron {
            hour: None,
            minute: 5,
            second: 0,
        }
    );
}

#[test]
fn an_unsupported_frequency_returns_the_schedule_error() {
    let recorder = Recorder::new(false);
    let mut params = support::params();
    params.frequency = Some("weekly".to_owned());
    let logger = support::build(&BTreeMap::new(), params, &recorder, None);
    let store = VantageLoggers::new();
    let registered: Arc<dyn VantageExportLogger> = Arc::new(logger);
    store.register(registered, VantageCallbackList::Callbacks);
    let scheduler = RecordingScheduler::new(false);
    let error = init_vantage_background_job(&store, &scheduler).expect_err("weekly is unsupported");
    assert!(matches!(
        &error,
        VantageInitError::Schedule(ScheduleError::UnsupportedFrequency { frequency })
            if frequency == "weekly"
    ));
    assert!(scheduler.jobs().is_empty());
}

#[test]
fn a_refused_registration_returns_the_scheduler_error() {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &BTreeMap::new(),
        support::params(),
        &recorder,
        None::<Arc<dyn FocusPodLock>>,
    );
    let store = VantageLoggers::new();
    let registered: Arc<dyn VantageExportLogger> = Arc::new(logger);
    store.register(registered, VantageCallbackList::Callbacks);
    let scheduler = RecordingScheduler::new(true);
    let error = init_vantage_background_job(&store, &scheduler).expect_err("the scheduler refuses");
    assert!(matches!(&error, VantageInitError::Scheduler(_)));
    assert!(scheduler.jobs().is_empty());
}
