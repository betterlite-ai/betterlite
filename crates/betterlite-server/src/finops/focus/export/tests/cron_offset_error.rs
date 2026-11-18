#![cfg(test)]

use std::sync::Arc;

use rstest::rstest;
use tokio_util::task::TaskTracker;

use crate::finops::focus::error::FocusError;
use crate::finops::focus::export::{FocusLogger, FocusLoggerDeps};
use crate::finops::focus::support::{self, FIXED_NOW, FixedClock, Recorder, at};

#[test]
fn an_unreadable_cron_offset_fails_construction() {
    let recorder = Recorder::new(false);
    let Err(error) = FocusLogger::new(
        &support::environment(&[("FOCUS_CRON_OFFSET", "soon")]),
        support::params(),
        FocusLoggerDeps {
            factory: recorder.factory(),
            clock: Arc::new(FixedClock { now: at(FIXED_NOW) }),
            pod_lock: None,
            tasks: TaskTracker::new(),
        },
    ) else {
        panic!("an unreadable offset must fail construction");
    };
    assert!(matches!(&error, FocusError::Environment(_)));
    assert_eq!(
        error.to_string(),
        "environment variable FOCUS_CRON_OFFSET must be a number"
    );
}

#[test]
fn a_parameter_does_not_rescue_an_unreadable_environment_offset() {
    let recorder = Recorder::new(false);
    let mut params = support::params();
    params.cron_offset_minute = Some(9);
    let Err(error) = FocusLogger::new(
        &support::environment(&[("FOCUS_CRON_OFFSET", "soon")]),
        params,
        FocusLoggerDeps {
            factory: recorder.factory(),
            clock: Arc::new(FixedClock { now: at(FIXED_NOW) }),
            pod_lock: None,
            tasks: TaskTracker::new(),
        },
    ) else {
        panic!("the environment is read before the parameter");
    };
    assert!(matches!(&error, FocusError::Environment(_)));
}

#[rstest]
#[case::positive("17", 17)]
#[case::negative("-3", -3)]
#[case::zero("0", 0)]
fn a_readable_cron_offset_is_used(#[case] value: &str, #[case] expected: i64) {
    let recorder = Recorder::new(false);
    let logger = support::build(
        &support::environment(&[("FOCUS_CRON_OFFSET", value)]),
        support::params(),
        recorder.factory(),
        None,
    );
    assert_eq!(logger.cron_offset_minute, expected);
}
