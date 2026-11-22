use betterlite_config::sections::settings::JsonValue;
use chrono::{TimeDelta, TimeZone as _, Utc};

use crate::cleanup_schedule::trigger::cleanup_trigger;
use crate::cleanup_schedule::{CLEANUP_CRON_KEY, RETENTION_INTERVAL_KEY, RETENTION_PERIOD_KEY};

use super::support::{capture_logs, fixture, origin, period_of, retained, store};

#[test]
fn cron_wins_over_the_interval() {
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (CLEANUP_CRON_KEY, JsonValue::from("* * * * *")),
        (RETENTION_INTERVAL_KEY, JsonValue::from("1d")),
    ]);
    let trigger = cleanup_trigger(&settings, &fixture.context()).unwrap();
    let first = trigger.next_fire_time(None, origin()).unwrap();
    let gap = first.signed_duration_since(origin());
    assert!(gap >= TimeDelta::zero());
    assert!(gap <= TimeDelta::seconds(60), "{gap}");
}

#[test]
fn bad_cron_string_returns_none_and_logs_the_value() {
    let (buffer, _guard) = capture_logs();
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (CLEANUP_CRON_KEY, JsonValue::from("not a cron")),
        (RETENTION_INTERVAL_KEY, JsonValue::from("1d")),
    ]);
    assert!(cleanup_trigger(&settings, &fixture.context()).is_none());
    assert!(
        buffer
            .contents()
            .contains("Invalid maximum_spend_logs_cleanup_cron value: not a cron")
    );
}

#[test]
fn non_string_cron_returns_none_and_logs_the_value() {
    let (buffer, _guard) = capture_logs();
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (CLEANUP_CRON_KEY, JsonValue::from(12_i64)),
        (RETENTION_INTERVAL_KEY, JsonValue::from("1d")),
    ]);
    assert!(cleanup_trigger(&settings, &fixture.context()).is_none());
    assert!(
        buffer
            .contents()
            .contains("Invalid maximum_spend_logs_cleanup_cron value: 12")
    );
}

#[test]
fn a_null_cron_falls_back_to_the_interval() {
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (CLEANUP_CRON_KEY, JsonValue::Null),
        (RETENTION_INTERVAL_KEY, JsonValue::from("90s")),
    ]);
    let trigger = cleanup_trigger(&settings, &fixture.context()).unwrap();
    assert_eq!(
        period_of(trigger.as_ref(), origin()),
        Some(TimeDelta::seconds(90))
    );
}

#[test]
fn an_empty_cron_string_is_logged_as_an_invalid_cron() {
    let (buffer, _guard) = capture_logs();
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (CLEANUP_CRON_KEY, JsonValue::from("")),
        (RETENTION_INTERVAL_KEY, JsonValue::from("90s")),
    ]);
    assert!(cleanup_trigger(&settings, &fixture.context()).is_none());
    assert!(
        buffer
            .contents()
            .contains("Invalid maximum_spend_logs_cleanup_cron value:")
    );
}

#[test]
fn default_interval_is_one_day() {
    let fixture = fixture();
    let settings = retained("30d");
    let trigger = cleanup_trigger(&settings, &fixture.context()).unwrap();
    assert_eq!(
        period_of(trigger.as_ref(), origin()),
        Some(TimeDelta::seconds(86_400))
    );
}

#[test]
fn interval_seconds_become_the_period() {
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (RETENTION_INTERVAL_KEY, JsonValue::from("2h")),
    ]);
    let trigger = cleanup_trigger(&settings, &fixture.context()).unwrap();
    assert_eq!(
        period_of(trigger.as_ref(), origin()),
        Some(TimeDelta::seconds(7_200))
    );
}

#[test]
fn non_string_interval_returns_none_and_logs_the_value() {
    let (buffer, _guard) = capture_logs();
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (RETENTION_INTERVAL_KEY, JsonValue::from(86_400_i64)),
    ]);
    assert!(cleanup_trigger(&settings, &fixture.context()).is_none());
    assert!(
        buffer
            .contents()
            .contains("Invalid maximum_spend_logs_retention_interval value: 86400")
    );
}

#[test]
fn unparsable_interval_returns_none_and_logs_the_value() {
    let (buffer, _guard) = capture_logs();
    let fixture = fixture();
    let settings = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (RETENTION_INTERVAL_KEY, JsonValue::from("soon")),
    ]);
    assert!(cleanup_trigger(&settings, &fixture.context()).is_none());
    assert!(
        buffer
            .contents()
            .contains("Invalid maximum_spend_logs_retention_interval value: \"soon\"")
    );
}

#[test]
fn python_datetime_max_is_the_end_of_year_9999() {
    let maximum = Utc.with_ymd_and_hms(9999, 12, 31, 23, 59, 59).unwrap();
    assert_eq!(maximum.timestamp(), 253_402_300_799);
}

#[test]
fn an_interval_past_python_datetime_is_rejected() {
    let fixture = fixture();
    let accepted = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (RETENTION_INTERVAL_KEY, JsonValue::from("251635075169s")),
    ]);
    let trigger = cleanup_trigger(&accepted, &fixture.context()).unwrap();
    assert_eq!(
        period_of(trigger.as_ref(), origin()),
        Some(TimeDelta::seconds(251_635_075_169))
    );
    let rejected = store(&[
        (RETENTION_PERIOD_KEY, JsonValue::from("30d")),
        (RETENTION_INTERVAL_KEY, JsonValue::from("251635075170s")),
    ]);
    let (buffer, _guard) = capture_logs();
    assert!(cleanup_trigger(&rejected, &fixture.context()).is_none());
    assert!(
        buffer
            .contents()
            .contains("Invalid maximum_spend_logs_retention_interval value:")
    );
}
