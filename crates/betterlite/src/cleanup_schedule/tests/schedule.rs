use betterlite_config::sections::settings::JsonValue;

use crate::cleanup_schedule::schedule::{
    CleanupSchedule, cleanup_schedule_of, is_truthy, value_text, wants_cleanup_job,
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
fn wants_cleanup_job_only_counts_the_first_four_keys() {
    let mut schedule = CleanupSchedule::default();
    assert!(!wants_cleanup_job(&schedule));
    for key in [
        RETENTION_PERIOD_KEY,
        AUTOROUTER_RETENTION_KEY,
        HEALTH_RETENTION_KEY,
        TAG_SPEND_RETENTION_KEY,
    ] {
        schedule = cleanup_schedule_of(&store(&[(key, JsonValue::from("30d"))]));
        assert!(wants_cleanup_job(&schedule), "{key} must want the job");
    }
    for key in [CLEANUP_CRON_KEY, RETENTION_INTERVAL_KEY] {
        schedule = cleanup_schedule_of(&store(&[(key, JsonValue::from("30d"))]));
        assert!(!wants_cleanup_job(&schedule), "{key} alone must not");
    }
}

#[test]
fn is_truthy_matches_python_truthiness() {
    let cases = [
        (JsonValue::Null, false),
        (JsonValue::from(true), true),
        (JsonValue::from(false), false),
        (JsonValue::from(0_i64), false),
        (JsonValue::from(7_i64), true),
        (JsonValue::from(-1_i64), true),
        (JsonValue::from(0.0_f64), false),
        (JsonValue::from(1.5_f64), true),
        (JsonValue::from(""), false),
        (JsonValue::from("0"), true),
        (serde_json::json!([]), false),
        (serde_json::json!([0]), true),
        (serde_json::json!({}), false),
        (serde_json::json!({"a": 0}), true),
    ];
    for (value, expected) in cases {
        assert_eq!(is_truthy(&value), expected, "{value}");
    }
}

#[test]
fn value_text_keeps_strings_raw_and_renders_other_values_as_json() {
    assert_eq!(value_text(&JsonValue::from("0 3 * * *")), "0 3 * * *");
    assert_eq!(value_text(&JsonValue::from(12_i64)), "12");
    assert_eq!(value_text(&JsonValue::from(true)), "true");
}
