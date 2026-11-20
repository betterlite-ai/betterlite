use std::error::Error as StdError;
use std::fmt::Write as _;

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::{AlertType, SlackAlertingArgs};
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_logging::alert_dispatch::{Alert, AlertLevel};
use betterlite_logging::analytics::python::float_text;
use chrono::{Days, Months, NaiveDate};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use crate::spend::estimates::report::{
    ReportDateError, SpendReportSource, SpendRow, spend_report_for_time_range,
};

mod tests;

pub(crate) const DEFAULT_TIME_RANGE: &str = "7d";
const SPEND_REPORT_PLACES: u32 = 4;
const MONTHLY_TTL_DAYS: u64 = 30;
const SECONDS_IN_AN_HOUR: u64 = 3_600;
const SECONDS_IN_A_DAY: u64 = 86_400;

#[async_trait]
pub trait SpendAlertCache: Send + Sync {
    async fn get(&self, key: &str) -> Option<String>;

    async fn set(&self, key: &str, value: &str, ttl_seconds: u64);
}

#[async_trait]
pub trait SpendAlertSender: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn send_alert(&self, alert: Alert<'_>) -> Result<(), Self::Error>;
}

// the weekly and monthly reports read the local date, the user spend pass the UTC date
pub trait SpendAlertClock: Send + Sync {
    fn local_date(&self) -> NaiveDate;

    fn utc_date(&self) -> NaiveDate;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TimeRangeError {
    #[error("{value:?} is not a number of days followed by d")]
    Days { value: String },
}

#[derive(Debug, thiserror::Error)]
pub enum SpendReportError<E>
where
    E: StdError + 'static,
{
    #[error("the {days}-day report window starts before the calendar supports")]
    Window { days: u32 },
    #[error("the month window could not be built")]
    MonthWindow,
    #[error("the report dates could not be read")]
    Dates(#[source] ReportDateError),
    #[error("the spend report could not be sent")]
    Send(#[source] E),
}

pub struct SpendReports<C, S, K> {
    pub alerting: Option<Vec<AlertDestination>>,
    pub alert_types: Vec<AlertType>,
    pub alerting_args: SlackAlertingArgs,
    pub hours_in_a_day: u32,
    pub cache: C,
    pub sender: S,
    pub source: Option<Box<dyn SpendReportSource>>,
    pub clock: K,
}

// LiteLLM keeps int(time_range[:-1]) and rejects a last character other than d or D
#[must_use]
pub(crate) fn parse_days(text: &str) -> Result<u32, TimeRangeError> {
    let mut characters = text.chars();
    let Some(unit) = characters.next_back() else {
        return Err(TimeRangeError::Days {
            value: text.to_owned(),
        });
    };
    if unit != 'd' && unit != 'D' {
        return Err(TimeRangeError::Days {
            value: text.to_owned(),
        });
    }
    characters
        .as_str()
        .parse::<u32>()
        .map_err(|_source| TimeRangeError::Days {
            value: text.to_owned(),
        })
}

// Python's round(float(x), places) rounds the exact binary value half to even, not the decimal text
pub(crate) fn float_rounded(amount: Decimal, places: u32) -> Decimal {
    Decimal::from_f64_retain(amount.as_f64())
        .unwrap_or(amount)
        .round_dp_with_strategy(places, RoundingStrategy::MidpointNearestEven)
}

impl<C, S, K> SpendReports<C, S, K>
where
    C: SpendAlertCache,
    S: SpendAlertSender,
    K: SpendAlertClock,
{
    pub async fn send_weekly_spend_report(&self, time_range: Option<&str>) {
        if self.alerting.is_none() || !self.alert_types.contains(&AlertType::SpendReports) {
            return;
        }
        let time_range = time_range.unwrap_or(DEFAULT_TIME_RANGE);
        let days = match parse_days(time_range) {
            Ok(days) => days,
            Err(error) => {
                tracing::error!("Invalid time range format: {error}");
                return;
            }
        };
        if let Err(error) = self.weekly_report(days).await {
            tracing::error!("Error sending spend report: {error}");
        }
    }

    pub async fn send_monthly_spend_report(&self) {
        if let Err(error) = self.monthly_report().await {
            tracing::error!("Error sending monthly spend report: {error}");
        }
    }

    async fn weekly_report(&self, days: u32) -> Result<(), SpendReportError<S::Error>> {
        let today = self.clock.local_date();
        let start = today
            .checked_sub_days(Days::new(u64::from(days)))
            .ok_or(SpendReportError::Window { days })?;
        let start_text = start.format("%Y-%m-%d").to_string();
        let today_text = today.format("%Y-%m-%d").to_string();
        let marker = format!("weekly_spend_report_sent_{start_text}_{today_text}");
        if self.cache.get(&marker).await.is_some() {
            return Ok(());
        }
        let report = spend_report_for_time_range(self.source.as_deref(), &start_text, &today_text)
            .await
            .map_err(SpendReportError::Dates)?;
        let Some((teams, tags)) = report else {
            return Ok(());
        };
        if teams.is_empty() && tags.is_empty() {
            return Ok(());
        }
        let message = weekly_message(start, today, days, &teams, &tags);
        self.send_report(&message).await?;
        let ttl = u64::from(days).saturating_mul(SECONDS_IN_A_DAY);
        self.cache.set(&marker, "SENT", ttl).await;
        Ok(())
    }

    async fn monthly_report(&self) -> Result<(), SpendReportError<S::Error>> {
        let today = self.clock.local_date();
        let first = today.with_day(1).ok_or(SpendReportError::MonthWindow)?;
        let next_month = first
            .checked_add_months(Months::new(1))
            .ok_or(SpendReportError::MonthWindow)?;
        let last = next_month.pred_opt().ok_or(SpendReportError::MonthWindow)?;
        let first_text = first.format("%Y-%m-%d").to_string();
        let last_text = last.format("%Y-%m-%d").to_string();
        let marker = format!("monthly_spend_report_sent_{first_text}_{last_text}");
        if self.cache.get(&marker).await.is_some() {
            return Ok(());
        }
        let report = spend_report_for_time_range(self.source.as_deref(), &first_text, &last_text)
            .await
            .map_err(SpendReportError::Dates)?;
        let Some((teams, tags)) = report else {
            return Ok(());
        };
        if teams.is_empty() && tags.is_empty() {
            return Ok(());
        }
        let message = monthly_message(first, last, &teams, &tags);
        self.send_report(&message).await?;
        let ttl = u64::from(self.hours_in_a_day)
            .saturating_mul(SECONDS_IN_AN_HOUR)
            .saturating_mul(MONTHLY_TTL_DAYS);
        self.cache.set(&marker, "SENT", ttl).await;
        Ok(())
    }

    async fn send_report(&self, message: &str) -> Result<(), SpendReportError<S::Error>> {
        let metadata = IndexMap::new();
        let alert = Alert {
            message,
            level: AlertLevel::Low,
            alert_type: AlertType::SpendReports,
            alerting_metadata: &metadata,
            user_info: None,
            request_model: None,
            api_base: None,
            kwargs: &metadata,
        };
        self.sender
            .send_alert(alert)
            .await
            .map_err(SpendReportError::Send)
    }
}

fn weekly_message(
    start: NaiveDate,
    end: NaiveDate,
    days: u32,
    teams: &[SpendRow],
    tags: &[SpendRow],
) -> String {
    let mut message = format!(
        "*💸 Spend Report for `{} - {}` ({days} days)*\n",
        start.format("%m-%d-%Y"),
        end.format("%m-%d-%Y"),
    );
    push_sections(&mut message, teams, tags);
    message
}

fn monthly_message(
    first: NaiveDate,
    last: NaiveDate,
    teams: &[SpendRow],
    tags: &[SpendRow],
) -> String {
    let mut message = format!(
        "*💸 Monthly Spend Report for `{} - {}` *\n",
        first.format("%m-%d-%Y"),
        last.format("%m-%d-%Y"),
    );
    push_sections(&mut message, teams, tags);
    message
}

// LiteLLM writes both headings even for an empty list, and a missing alias as None
fn push_sections(message: &mut String, teams: &[SpendRow], tags: &[SpendRow]) {
    push_rows(message, "Team", teams);
    push_rows(message, "Tag", tags);
}

fn push_rows(message: &mut String, label: &str, rows: &[SpendRow]) {
    let _ignored = writeln!(message, "\n*{label} Spend Report:*");
    for row in rows {
        let spend = float_text(float_rounded(row.total_spend, SPEND_REPORT_PLACES).as_f64());
        let name = row.name.as_deref().unwrap_or("None");
        let _ignored = writeln!(message, "{label}: `{name}` | Spend: `${spend}`");
    }
}
