#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use super::{JobDefaults, ShutdownTimeouts};
use crate::job_scheduler::SchedulerError;

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn job_defaults_read_litellm_defaults() {
    let defaults = JobDefaults::from_env(&BTreeMap::new()).unwrap();
    assert!(defaults.coalesce);
    assert_eq!(defaults.misfire_grace_seconds, 3600);
    assert_eq!(defaults.max_instances, 1);
}

#[test]
fn job_defaults_read_the_environment() {
    let defaults = JobDefaults::from_env(&env(&[
        ("APSCHEDULER_COALESCE", "False"),
        ("APSCHEDULER_MISFIRE_GRACE_TIME", "120"),
        ("APSCHEDULER_MAX_INSTANCES", "3"),
    ]))
    .unwrap();
    assert!(!defaults.coalesce);
    assert_eq!(defaults.misfire_grace_seconds, 120);
    assert_eq!(defaults.max_instances, 3);
}

#[rstest]
#[case::true_word("true", true)]
#[case::one("1", true)]
#[case::upper_case("TRUE", true)]
#[case::false_word("false", false)]
#[case::zero("0", false)]
fn job_defaults_read_coalesce_as_a_bool(#[case] text: &str, #[case] expected: bool) {
    let defaults = JobDefaults::from_env(&env(&[("APSCHEDULER_COALESCE", text)])).unwrap();
    assert_eq!(defaults.coalesce, expected);
}

#[rstest]
#[case::grace("APSCHEDULER_MISFIRE_GRACE_TIME", "soon")]
#[case::instances("APSCHEDULER_MAX_INSTANCES", "many")]
#[case::coalesce("APSCHEDULER_COALESCE", "perhaps")]
fn job_defaults_reject_an_invalid_value(#[case] name: &str, #[case] value: &str) {
    let error = JobDefaults::from_env(&env(&[(name, value)])).unwrap_err();
    assert!(matches!(error, SchedulerError::InvalidEnv { .. }));
    assert!(error.to_string().contains(name));
}

#[test]
fn shutdown_timeouts_read_litellm_defaults() {
    let timeouts = ShutdownTimeouts::from_env(&BTreeMap::new()).unwrap();
    assert_eq!(timeouts.finish, Duration::from_secs(5));
    assert_eq!(timeouts.cancel, Duration::from_secs(5));
}

#[test]
fn shutdown_timeouts_read_the_environment() {
    let timeouts = ShutdownTimeouts::from_env(&env(&[
        ("SCHEDULED_JOB_SHUTDOWN_FINISH_TIMEOUT_SECONDS", "2.5"),
        ("SCHEDULED_JOB_SHUTDOWN_CANCEL_TIMEOUT_SECONDS", "1"),
    ]))
    .unwrap();
    assert_eq!(timeouts.finish, Duration::from_millis(2500));
    assert_eq!(timeouts.cancel, Duration::from_secs(1));
}

#[rstest]
#[case::finish("SCHEDULED_JOB_SHUTDOWN_FINISH_TIMEOUT_SECONDS", "later")]
#[case::cancel("SCHEDULED_JOB_SHUTDOWN_CANCEL_TIMEOUT_SECONDS", "-3")]
fn shutdown_timeouts_reject_an_invalid_value(#[case] name: &str, #[case] value: &str) {
    let error = ShutdownTimeouts::from_env(&env(&[(name, value)])).unwrap_err();
    assert!(matches!(error, SchedulerError::InvalidEnv { .. }));
    assert!(error.to_string().contains(name));
}
