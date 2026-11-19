#![cfg(test)]

use std::sync::Arc;

use rstest::rstest;
use tracing::Level;

use crate::finops::focus::export_schedule::ScheduleTrigger;
use crate::finops::mavvrik::schedule::{
    MavvrikLoggerType, RegisteredMavvrikLogger, init_mavvrik_background_job,
};
use crate::finops::mavvrik::support::{
    FakeFactory, FakeLazyInit, Recorder, RecordingScheduler, build, capture_logs, environment,
};

#[rstest]
#[case::default_offset(None, 0, 5)]
#[case::past_an_hour(Some("90"), 1, 30)]
#[case::past_a_day(Some("2000"), 23, 20)]
#[case::negative(Some("-10"), 0, 0)]
fn the_job_runs_daily_at_the_configured_offset(
    #[case] offset: Option<&str>,
    #[case] hour: i64,
    #[case] minute: i64,
) {
    let recorder = Recorder::default();
    let entries: Vec<(&str, &str)> = offset
        .map(|offset| vec![("FOCUS_CRON_OFFSET", offset)])
        .unwrap_or_default();
    let logger = Arc::new(build(&environment(&entries), &recorder, None));
    let registered = vec![RegisteredMavvrikLogger {
        logger: Arc::clone(&logger),
        logger_type: MavvrikLoggerType::Mavvrik,
    }];
    let lazy_init = FakeLazyInit::new(None);
    let scheduler = RecordingScheduler::new(false);
    init_mavvrik_background_job(&registered, false, &lazy_init, &scheduler)
        .expect("the job is scheduled");
    let jobs = scheduler.jobs();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].1, "mavvrik_focus_export_usage_data");
    assert_eq!(
        jobs[0].2,
        ScheduleTrigger::Cron {
            hour: Some(hour),
            minute,
            second: 0,
        }
    );
    assert!(jobs[0].3);
    assert!(Arc::ptr_eq(&jobs[0].0, &logger));
}

#[test]
fn the_registration_logs_the_trigger() {
    let (logs, _guard) = capture_logs();
    let logger = Arc::new(build(&environment(&[]), &Recorder::default(), None));
    let registered = vec![RegisteredMavvrikLogger {
        logger,
        logger_type: MavvrikLoggerType::Mavvrik,
    }];
    let lazy_init = FakeLazyInit::<FakeFactory>::new(None);
    let scheduler = RecordingScheduler::new(false);
    init_mavvrik_background_job(&registered, false, &lazy_init, &scheduler)
        .expect("the job is scheduled");
    assert!(logs.has_at(
        Level::INFO,
        "mavvrik_focus: background export job scheduled"
    ));
}

#[test]
fn a_scheduler_that_rejects_the_job_fails_the_init() {
    let logger = Arc::new(build(&environment(&[]), &Recorder::default(), None));
    let registered = vec![RegisteredMavvrikLogger {
        logger,
        logger_type: MavvrikLoggerType::Mavvrik,
    }];
    let lazy_init = FakeLazyInit::<FakeFactory>::new(None);
    let scheduler = RecordingScheduler::new(true);
    let error = init_mavvrik_background_job(&registered, false, &lazy_init, &scheduler)
        .expect_err("the scheduler rejects the job");
    assert!(error.to_string().contains("could not be scheduled"));
}
