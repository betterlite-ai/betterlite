use std::error::Error as StdError;
use std::ops::{Div, Mul};

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_logging::analytics::python::float_text;
use chrono::NaiveDate;
use indexmap::IndexMap;
use serde::Serialize;

use super::spend_reports::{SpendAlertCache, SpendAlertClock, SpendAlertSender};

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

// LiteLLM keeps these as f64 from a ::float cast, so the comparisons and the .2f text follow the binary value
#[derive(Clone, Debug, PartialEq)]
pub struct UserSpendRow {
    pub user_id: String,
    pub daily_spend: f64,
    pub monthly_spend: f64,
    pub baseline_spend: f64,
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

pub fn evaluate_user_spend(
    row: &UserSpendRow,
    args: &SlackAlertingArgs,
    today: NaiveDate,
    checks: UserSpendChecks,
    hours_in_a_day: u32,
) -> Vec<UserSpendAlertEvent> {
    let mut events = Vec::new();
    if checks.thresholds {
        events.extend(daily_threshold_event(row, args, today, hours_in_a_day));
        events.extend(monthly_threshold_event(row, args, today, hours_in_a_day));
    }
    if checks.anomalies {
        events.extend(anomaly_event(row, args, today, hours_in_a_day));
    }
    events
}

fn day_seconds(hours_in_a_day: u32) -> u64 {
    u64::from(hours_in_a_day).saturating_mul(SECONDS_IN_AN_HOUR)
}

fn daily_threshold_event(
    row: &UserSpendRow,
    args: &SlackAlertingArgs,
    today: NaiveDate,
    hours_in_a_day: u32,
) -> Option<UserSpendAlertEvent> {
    let threshold = args.daily_user_spend_threshold?.to_f64()?;
    if row.daily_spend < threshold {
        return None;
    }
    let today_text = today.format("%Y-%m-%d").to_string();
    Some(UserSpendAlertEvent {
        kind: UserSpendAlertKind::DailyThreshold,
        alert_type: AlertType::UserSpendThresholds,
        message: format!(
            "User Daily Spend Threshold Crossed:\nUser: `{}`\nSpend Today: `${:.2}`\nDaily Threshold: `${:.2}`",
            row.user_id, row.daily_spend, threshold,
        ),
        cache_key: format!("user_spend_alert_daily_{}_{today_text}", row.user_id),
        cache_ttl: day_seconds(hours_in_a_day),
    })
}

fn monthly_threshold_event(
    row: &UserSpendRow,
    args: &SlackAlertingArgs,
    today: NaiveDate,
    hours_in_a_day: u32,
) -> Option<UserSpendAlertEvent> {
    let threshold = args.monthly_user_spend_threshold?.to_f64()?;
    if row.monthly_spend < threshold {
        return None;
    }
    let month_text = today.format("%Y-%m").to_string();
    Some(UserSpendAlertEvent {
        kind: UserSpendAlertKind::MonthlyThreshold,
        alert_type: AlertType::UserSpendThresholds,
        message: format!(
            "User Monthly Spend Threshold Crossed:\nUser: `{}`\nSpend This Month: `${:.2}`\nMonthly Threshold: `${:.2}`",
            row.user_id, row.monthly_spend, threshold,
        ),
        cache_key: format!("user_spend_alert_monthly_{}_{month_text}", row.user_id),
        cache_ttl: day_seconds(hours_in_a_day).saturating_mul(MONTHLY_TTL_DAYS),
    })
}

fn anomaly_event(
    row: &UserSpendRow,
    args: &SlackAlertingArgs,
    today: NaiveDate,
    hours_in_a_day: u32,
) -> Option<UserSpendAlertEvent> {
    let minimum = args.spend_anomaly_min_spend.to_f64()?;
    if row.daily_spend < minimum {
        return None;
    }
    let days = f64::from(args.spend_anomaly_baseline_days);
    let average = Div::div(row.baseline_spend, days);
    let multiplier = args.spend_anomaly_multiplier.to_f64()?;
    if row.baseline_spend > 0.0 && row.daily_spend <= Mul::mul(multiplier, average) {
        return None;
    }
    let today_text = today.format("%Y-%m-%d").to_string();
    Some(UserSpendAlertEvent {
        kind: UserSpendAlertKind::Anomaly,
        alert_type: AlertType::UserSpendAnomalies,
        message: format!(
            "User Spend Anomaly Detected:\nUser: `{}`\nSpend Today: `${:.2}`\nDaily Average (last {} days): `${:.2}`\nTrigger: spend above `{}x` the daily average (minimum `${:.2}`)",
            row.user_id,
            row.daily_spend,
            args.spend_anomaly_baseline_days,
            average,
            float_text(multiplier),
            minimum,
        ),
        cache_key: format!("user_spend_alert_anomaly_{}_{today_text}", row.user_id),
        cache_ttl: day_seconds(hours_in_a_day),
    })
}

impl<C, S, R, K> UserSpendAlerts<C, S, R, K>
where
    C: SpendAlertCache,
    S: SpendAlertSender,
    R: UserSpendSource,
    K: SpendAlertClock,
{
    pub async fn send_user_spend_alerts(&self, has_database: bool) {
        if let Err(error) = self.send_pass(has_database).await {
            tracing::error!("Error sending user spend alerts: {error}");
        }
    }

    async fn send_pass(
        &self,
        has_database: bool,
    ) -> Result<(), UserSpendAlertError<R::Error, S::Error>> {
        let Some(destinations) = self.alerting.as_deref() else {
            return Ok(());
        };
        if !destinations.contains(&AlertDestination::Slack) {
            return Ok(());
        }
        let checks = UserSpendChecks {
            thresholds: self.alert_types.contains(&AlertType::UserSpendThresholds),
            anomalies: self.alert_types.contains(&AlertType::UserSpendAnomalies),
        };
        if !checks.thresholds && !checks.anomalies {
            return Ok(());
        }
        if !has_database {
            return Ok(());
        }
        let today = self.clock.utc_date();
        let rows = self
            .source
            .rows(today, self.alerting_args.spend_anomaly_baseline_days)
            .await
            .map_err(UserSpendAlertError::Rows)?;
        let mut events = Vec::new();
        for row in &rows {
            events.extend(evaluate_user_spend(
                row,
                &self.alerting_args,
                today,
                checks,
                self.hours_in_a_day,
            ));
        }
        let mut new_events = Vec::new();
        for event in events {
            if self.cache.get(&event.cache_key).await.is_none() {
                new_events.push(event);
            }
        }
        for alert_type in [
            AlertType::UserSpendThresholds,
            AlertType::UserSpendAnomalies,
        ] {
            let typed: Vec<&UserSpendAlertEvent> = new_events
                .iter()
                .filter(|event| event.alert_type == alert_type)
                .collect();
            if typed.is_empty() {
                continue;
            }
            let message = typed
                .iter()
                .map(|event| event.message.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let metadata = IndexMap::new();
            let alert = Alert {
                message: &message,
                level: AlertLevel::High,
                alert_type,
                alerting_metadata: &metadata,
                user_info: None,
                request_model: None,
                api_base: None,
                kwargs: &metadata,
            };
            self.sender
                .send_alert(alert)
                .await
                .map_err(UserSpendAlertError::Send)?;
            for event in typed {
                self.cache
                    .set(&event.cache_key, "SENT", event.cache_ttl)
                    .await;
            }
        }
        Ok(())
    }
}
