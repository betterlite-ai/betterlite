use std::error::Error as StdError;
use std::ops::{Div, Mul};

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_logging::analytics::python::float_text;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;

mod dispatch;
pub mod postgres;

#[path = "user_spend_alerts/tests/evaluate.rs"]
mod evaluate;
#[path = "user_spend_alerts/tests/query.rs"]
mod query;
#[path = "user_spend_alerts/tests/send.rs"]
mod send;
#[path = "user_spend_alerts/tests/support.rs"]
mod support;

const SECONDS_IN_AN_HOUR: u64 = 3_600;
const MONTHLY_TTL_DAYS: u64 = 32;

// LiteLLM reads these as a Postgres ::float, so to_f64 restores the binary value before each comparison and .2f text
#[derive(Clone, Debug, PartialEq)]
pub struct UserSpendRow {
    pub user_id: String,
    pub daily_spend: Decimal,
    pub monthly_spend: Decimal,
    pub baseline_spend: Decimal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UserSpendAlertKind {
    DailyThreshold,
    MonthlyThreshold,
    Anomaly,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UserSpendAlertEvent {
    pub kind: UserSpendAlertKind,
    pub alert_type: AlertType,
    pub message: String,
    pub cache_key: String,
    pub cache_ttl: u64,
}

// LiteLLM passes the two enable flags as parameters; the workspace allows one bool parameter per function
#[derive(Clone, Copy, Debug)]
pub struct UserSpendChecks {
    pub thresholds: bool,
    pub anomalies: bool,
}

#[async_trait]
pub trait UserSpendSource: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn rows(
        &self,
        today: NaiveDate,
        baseline_days: u32,
    ) -> Result<Vec<UserSpendRow>, Self::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum UserSpendAlertError<SourceError, SendError>
where
    SourceError: StdError + 'static,
    SendError: StdError + 'static,
{
    #[error("the user spend rows could not be read")]
    Rows(#[source] SourceError),
    #[error("the user spend alerts could not be sent")]
    Send(#[source] SendError),
}

pub struct UserSpendAlerts<C, S, R, K> {
    pub alerting: Option<Vec<AlertDestination>>,
    pub alert_types: Vec<AlertType>,
    pub alerting_args: SlackAlertingArgs,
    pub hours_in_a_day: u32,
    pub cache: C,
    pub sender: S,
    pub source: R,
    pub clock: K,
}

#[must_use]
pub fn evaluate_user_spend(
    row: &UserSpendRow,
    args: &SlackAlertingArgs,
    today: NaiveDate,
    checks: UserSpendChecks,
    hours_in_a_day: u32,
) -> Vec<UserSpendAlertEvent> {
    let mut events = Vec::new();
    if checks.thresholds {
        events.extend(threshold_event(
            row,
            args,
            today,
            hours_in_a_day,
            ThresholdKind::Daily,
        ));
        events.extend(threshold_event(
            row,
            args,
            today,
            hours_in_a_day,
            ThresholdKind::Monthly,
        ));
    }
    if checks.anomalies {
        events.extend(anomaly_event(row, args, today, hours_in_a_day));
    }
    events
}

#[derive(Clone, Copy)]
enum ThresholdKind {
    Daily,
    Monthly,
}

struct ThresholdSpec {
    alert_kind: UserSpendAlertKind,
    header: &'static str,
    spend_label: &'static str,
    threshold_label: &'static str,
    key_prefix: &'static str,
    date_format: &'static str,
    ttl_days: u64,
}

const DAILY_THRESHOLD: ThresholdSpec = ThresholdSpec {
    alert_kind: UserSpendAlertKind::DailyThreshold,
    header: "Daily",
    spend_label: "Spend Today",
    threshold_label: "Daily Threshold",
    key_prefix: "user_spend_alert_daily",
    date_format: "%Y-%m-%d",
    ttl_days: 1,
};

const MONTHLY_THRESHOLD: ThresholdSpec = ThresholdSpec {
    alert_kind: UserSpendAlertKind::MonthlyThreshold,
    header: "Monthly",
    spend_label: "Spend This Month",
    threshold_label: "Monthly Threshold",
    key_prefix: "user_spend_alert_monthly",
    date_format: "%Y-%m",
    ttl_days: MONTHLY_TTL_DAYS,
};

fn threshold_event(
    row: &UserSpendRow,
    args: &SlackAlertingArgs,
    today: NaiveDate,
    hours_in_a_day: u32,
    kind: ThresholdKind,
) -> Option<UserSpendAlertEvent> {
    let (spec, threshold, spend) = match kind {
        ThresholdKind::Daily => (
            &DAILY_THRESHOLD,
            args.daily_user_spend_threshold,
            row.daily_spend,
        ),
        ThresholdKind::Monthly => (
            &MONTHLY_THRESHOLD,
            args.monthly_user_spend_threshold,
            row.monthly_spend,
        ),
    };
    let threshold_value = threshold?.to_f64()?;
    let spend_value = spend.to_f64()?;
    if spend_value < threshold_value {
        return None;
    }
    let date_text = today.format(spec.date_format).to_string();
    Some(UserSpendAlertEvent {
        kind: spec.alert_kind,
        alert_type: AlertType::UserSpendThresholds,
        message: format!(
            "User {} Spend Threshold Crossed:\nUser: `{}`\n{}: `${spend_value:.2}`\n{}: `${threshold_value:.2}`",
            spec.header, row.user_id, spec.spend_label, spec.threshold_label,
        ),
        cache_key: format!("{}_{}_{date_text}", spec.key_prefix, row.user_id),
        cache_ttl: u64::from(hours_in_a_day)
            .saturating_mul(SECONDS_IN_AN_HOUR)
            .saturating_mul(spec.ttl_days),
    })
}

fn anomaly_event(
    row: &UserSpendRow,
    args: &SlackAlertingArgs,
    today: NaiveDate,
    hours_in_a_day: u32,
) -> Option<UserSpendAlertEvent> {
    let daily = row.daily_spend.to_f64()?;
    let minimum = args.spend_anomaly_min_spend.to_f64()?;
    if daily < minimum {
        return None;
    }
    let baseline = row.baseline_spend.to_f64()?;
    let days = f64::from(args.spend_anomaly_baseline_days);
    let average = Div::div(baseline, days);
    let multiplier = args.spend_anomaly_multiplier.to_f64()?;
    if baseline > 0.0 && daily <= Mul::mul(multiplier, average) {
        return None;
    }
    let today_text = today.format("%Y-%m-%d").to_string();
    Some(UserSpendAlertEvent {
        kind: UserSpendAlertKind::Anomaly,
        alert_type: AlertType::UserSpendAnomalies,
        message: format!(
            "User Spend Anomaly Detected:\nUser: `{}`\nSpend Today: `${daily:.2}`\nDaily Average (last {} days): `{average:.2}`\nTrigger: spend above `{}x` the daily average (minimum `{minimum:.2}`)",
            row.user_id,
            args.spend_anomaly_baseline_days,
            float_text(multiplier),
        ),
        cache_key: format!("user_spend_alert_anomaly_{}_{today_text}", row.user_id),
        cache_ttl: u64::from(hours_in_a_day).saturating_mul(SECONDS_IN_AN_HOUR),
    })
}
