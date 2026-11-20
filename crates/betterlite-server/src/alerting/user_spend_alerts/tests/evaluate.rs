#![cfg(test)]

use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use rstest::rstest;

use super::support::{args, date, decimal, row};
use super::{
    UserSpendAlertEvent, UserSpendAlertKind, UserSpendChecks, UserSpendRow, evaluate_user_spend,
};

const TODAY: &str = "2026-01-15";

fn checks() -> UserSpendChecks {
    UserSpendChecks {
        thresholds: true,
        anomalies: true,
    }
}

fn threshold_only() -> UserSpendChecks {
    UserSpendChecks {
        thresholds: true,
        anomalies: false,
    }
}

fn anomaly_only() -> UserSpendChecks {
    UserSpendChecks {
        thresholds: false,
        anomalies: true,
    }
}

fn evaluate(
    row: &UserSpendRow,
    settings: &SlackAlertingArgs,
    checks: UserSpendChecks,
) -> Vec<UserSpendAlertEvent> {
    evaluate_user_spend(row, settings, date(TODAY), checks, 24)
}

#[test]
fn evaluate_daily_threshold_fires_at_equal_spend() {
    let mut settings = args();
    settings.daily_user_spend_threshold = Some(decimal("10"));
    let events = evaluate(&row("user-1", 10.0, 0.0, 0.0), &settings, threshold_only());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, UserSpendAlertKind::DailyThreshold);
    assert_eq!(events[0].alert_type, AlertType::UserSpendThresholds);
    assert_eq!(
        events[0].message,
        "User Daily Spend Threshold Crossed:\nUser: `user-1`\nSpend Today: `$10.00`\nDaily Threshold: `$10.00`"
    );
    assert_eq!(
        events[0].cache_key,
        "user_spend_alert_daily_user-1_2026-01-15"
    );
    assert_eq!(events[0].cache_ttl, 86_400);
}

#[rstest]
#[case::below_the_threshold(Some("10"), 9.999)]
#[case::no_threshold(None, 1_000.0)]
fn evaluate_daily_threshold_skips_below_and_unset(
    #[case] threshold: Option<&str>,
    #[case] daily: f64,
) {
    let mut settings = args();
    settings.daily_user_spend_threshold = threshold.map(decimal);
    let events = evaluate(&row("user-1", daily, 0.0, 0.0), &settings, threshold_only());
    assert!(events.is_empty());
}

#[test]
fn evaluate_monthly_threshold_fires_at_equal_spend() {
    let mut settings = args();
    settings.monthly_user_spend_threshold = Some(decimal("25"));
    let events = evaluate(&row("user-3", 0.0, 25.0, 0.0), &settings, threshold_only());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, UserSpendAlertKind::MonthlyThreshold);
    assert_eq!(events[0].alert_type, AlertType::UserSpendThresholds);
    assert_eq!(
        events[0].message,
        "User Monthly Spend Threshold Crossed:\nUser: `user-3`\nSpend This Month: `$25.00`\nMonthly Threshold: `$25.00`"
    );
    assert_eq!(
        events[0].cache_key,
        "user_spend_alert_monthly_user-3_2026-01"
    );
    assert_eq!(events[0].cache_ttl, 2_764_800);
}

#[rstest]
#[case::below_the_threshold(Some("25"), 24.999)]
#[case::no_threshold(None, 1_000.0)]
fn evaluate_monthly_threshold_skips_below_and_unset(
    #[case] threshold: Option<&str>,
    #[case] monthly: f64,
) {
    let mut settings = args();
    settings.monthly_user_spend_threshold = threshold.map(decimal);
    let events = evaluate(
        &row("user-3", 0.0, monthly, 0.0),
        &settings,
        threshold_only(),
    );
    assert!(events.is_empty());
}

#[test]
fn evaluate_thresholds_need_the_thresholds_flag() {
    let mut settings = args();
    settings.spend_anomaly_min_spend = decimal("1000");
    let events = evaluate(&row("user-1", 100.0, 100.0, 0.0), &settings, anomaly_only());
    assert!(events.is_empty());
}

#[test]
fn evaluate_anomaly_fires_above_the_average() {
    let settings = args();
    let events = evaluate(&row("user-1", 12.5, 40.0, 21.0), &settings, anomaly_only());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, UserSpendAlertKind::Anomaly);
    assert_eq!(events[0].alert_type, AlertType::UserSpendAnomalies);
    assert_eq!(
        events[0].message,
        "User Spend Anomaly Detected:\nUser: `user-1`\nSpend Today: `$12.50`\nDaily Average (last 7 days): `$3.00`\nTrigger: spend above `3.0x` the daily average (minimum `$10.00`)"
    );
    assert_eq!(
        events[0].cache_key,
        "user_spend_alert_anomaly_user-1_2026-01-15"
    );
    assert_eq!(events[0].cache_ttl, 86_400);
}

#[rstest]
#[case::at_the_boundary(300.0, 0)]
#[case::above_the_boundary(300.0001, 1)]
fn evaluate_anomaly_checks_the_multiplier_boundary(#[case] daily: f64, #[case] found: usize) {
    let settings = args();
    let events = evaluate(
        &row("user-4", daily, daily, 700.0),
        &settings,
        anomaly_only(),
    );
    assert_eq!(events.len(), found);
}

#[test]
fn evaluate_anomaly_fires_without_a_baseline_at_the_minimum() {
    let events = evaluate(&row("user-5", 10.0, 10.0, 0.0), &args(), anomaly_only());
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].message,
        "User Spend Anomaly Detected:\nUser: `user-5`\nSpend Today: `$10.00`\nDaily Average (last 7 days): `$0.00`\nTrigger: spend above `3.0x` the daily average (minimum `$10.00`)"
    );
}

// pydantic's gt=0 keeps a zero minimum out of LiteLLM, and the multiplier guard still only applies to a positive baseline
#[test]
fn evaluate_anomaly_fires_at_zero_spend_without_a_baseline() {
    let mut settings = args();
    settings.spend_anomaly_min_spend = decimal("0");
    let events = evaluate(&row("user-7", 0.0, 0.0, 0.0), &settings, anomaly_only());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, UserSpendAlertKind::Anomaly);
}

#[rstest]
#[case::below_the_minimum(9.999, 100.0, 0)]
#[case::at_the_minimum(10.0, 0.0, 1)]
fn evaluate_anomaly_needs_the_minimum_spend(
    #[case] daily: f64,
    #[case] baseline: f64,
    #[case] found: usize,
) {
    let events = evaluate(
        &row("user-6", daily, daily, baseline),
        &args(),
        anomaly_only(),
    );
    assert_eq!(events.len(), found);
}

#[test]
fn evaluate_anomaly_needs_the_anomalies_flag() {
    let mut settings = args();
    settings.spend_anomaly_min_spend = decimal("0");
    let events = evaluate(
        &row("user-1", 100.0, 100.0, 0.0),
        &settings,
        threshold_only(),
    );
    assert!(
        events
            .iter()
            .all(|event| event.alert_type == AlertType::UserSpendThresholds)
    );
}

#[test]
fn evaluate_returns_thresholds_before_anomalies() {
    let events = evaluate(&row("user-1", 12.5, 40.0, 21.0), &args(), checks());
    let kinds: Vec<UserSpendAlertKind> = events.iter().map(|event| event.kind).collect();
    assert_eq!(
        kinds,
        vec![
            UserSpendAlertKind::DailyThreshold,
            UserSpendAlertKind::MonthlyThreshold,
            UserSpendAlertKind::Anomaly,
        ]
    );
}

#[rstest]
#[case::binary_below_the_tie(2.675, "2.67")]
#[case::binary_above_the_tie(0.005, "0.01")]
#[case::rounds_up(12.345, "12.35")]
#[case::whole(10.0, "10.00")]
fn evaluate_rounds_money_like_python(#[case] daily: f64, #[case] shown: &str) {
    let mut settings = args();
    settings.daily_user_spend_threshold = Some(decimal("0"));
    let events = evaluate(&row("user-1", daily, 0.0, 0.0), &settings, threshold_only());
    assert_eq!(events.len(), 1);
    assert!(
        events[0]
            .message
            .contains(&format!("Spend Today: `${shown}`"))
    );
}

#[test]
fn evaluate_uses_hours_in_a_day_for_the_event_ttls() {
    let settings = args();
    let events = evaluate_user_spend(
        &row("user-1", 12.5, 40.0, 21.0),
        &settings,
        date(TODAY),
        checks(),
        1,
    );
    let ttls: Vec<u64> = events.iter().map(|event| event.cache_ttl).collect();
    assert_eq!(ttls, vec![3_600, 115_200, 3_600]);
}

#[rstest]
#[case::fractional("2.5", "2.5x")]
#[case::whole("3", "3.0x")]
fn evaluate_prints_the_multiplier_like_python(#[case] multiplier: &str, #[case] shown: &str) {
    let mut settings = args();
    settings.spend_anomaly_multiplier = decimal(multiplier);
    let events = evaluate(&row("user-1", 100.0, 100.0, 0.0), &settings, anomaly_only());
    assert_eq!(events.len(), 1);
    assert!(
        events[0]
            .message
            .contains(&format!("spend above `{shown}` the daily average"))
    );
}

#[test]
fn evaluate_prints_the_baseline_days_in_the_message() {
    let mut settings = args();
    settings.spend_anomaly_baseline_days = 3;
    let events = evaluate(
        &row("user-1", 100.0, 100.0, 30.0),
        &settings,
        anomaly_only(),
    );
    assert_eq!(events.len(), 1);
    assert!(
        events[0]
            .message
            .contains("Daily Average (last 3 days): `$10.00`")
    );
}
