use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use crate::cleanup_schedule::schedule::{
    CleanupSchedule, cleanup_schedule_of, value_text, wants_cleanup_job,
};
use crate::cleanup_schedule::{
    AUTOROUTER_RETENTION_KEY, CLEANUP_CRON_KEY, CLEANUP_SCHEDULE_KEYS, HEALTH_RETENTION_KEY,
    RETENTION_INTERVAL_KEY, RETENTION_PERIOD_KEY, TAG_SPEND_RETENTION_KEY,
};

use super::support::store;

#[test]
fn cleanup_schedule_keys_match_litellm_order() {
    assert_eq!(
        CLEANUP_SCHEDULE_KEYS,
        [
            "maximum_spend_logs_retention_period",
            "maximum_autorouter_session_retention_period",
            "maximum_health_check_retention_period",
            "maximum_daily_tag_spend_retention_period",
            "maximum_spend_logs_cleanup_cron",
            "maximum_spend_logs_retention_interval",
        ]
    );
}

#[test]
fn cleanup_schedule_reads_each_key_and_treats_null_as_absent() {
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (AUTOROUTER_RETENTION_KEY, JsonValue::Null),
        (HEALTH_RETENTION_KEY, JsonValue::from("7d")),
        (TAG_SPEND_RETENTION_KEY, JsonValue::from("90d")),
        (CLEANUP_CRON_KEY, JsonValue::from("0 3 * * *")),
        (RETENTION_INTERVAL_KEY, JsonValue::from("1h")),
    ]);
    let schedule = cleanup_schedule_of(&settings);
    assert_eq!(schedule.spend_logs_retention, Some(JsonValue::from("30d")));
    assert_eq!(schedule.autorouter_sessions_retention, None);
    assert_eq!(
        schedule.health_checks_retention,
        Some(JsonValue::from("7d"))
    );
    assert_eq!(schedule.tag_spend_retention, Some(JsonValue::from("90d")));
    assert_eq!(schedule.cleanup_cron, Some(JsonValue::from("0 3 * * *")));
    assert_eq!(schedule.cleanup_interval, Some(JsonValue::from("1h")));
    assert_eq!(cleanup_schedule_of(&store(&[])), CleanupSchedule::default());
}

#[test]
fn an_empty_schedule_does_not_want_the_job() {
    assert!(!wants_cleanup_job(&CleanupSchedule::default()));
}

#[rstest]
#[case::retention_period(RETENTION_PERIOD_KEY)]
#[case::autorouter_sessions(AUTOROUTER_RETENTION_KEY)]
#[case::health_checks(HEALTH_RETENTION_KEY)]
#[case::tag_spend(TAG_SPEND_RETENTION_KEY)]
fn a_retention_key_wants_the_job(#[case] key: &str) {
    let schedule = cleanup_schedule_of(&store(&[(key, JsonValue::from("30d"))]));
    assert!(wants_cleanup_job(&schedule), "{key} must want the job");
}

#[rstest]
#[case::cleanup_cron(CLEANUP_CRON_KEY)]
#[case::retention_interval(RETENTION_INTERVAL_KEY)]
fn a_schedule_key_alone_does_not_want_the_job(#[case] key: &str) {
    let schedule = cleanup_schedule_of(&store(&[(key, JsonValue::from("30d"))]));
    assert!(!wants_cleanup_job(&schedule), "{key} alone must not");
}

#[test]
fn value_text_keeps_strings_raw_and_renders_other_values_as_json() {
    assert_eq!(value_text(&JsonValue::from("0 3 * * *")), "0 3 * * *");
    assert_eq!(value_text(&JsonValue::from(12_i64)), "12");
    assert_eq!(value_text(&JsonValue::from(true)), "true");
}
