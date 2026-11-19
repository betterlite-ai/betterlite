use async_trait::async_trait;
use betterlite_db::postgres_store::errors::DbError;
use chrono::{DateTime, NaiveDate, TimeZone as _, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

pub mod postgres;

pub const DATABASE_NOT_CONNECTED: &str =
    "Database not connected. Connect a database to your proxy for weekly, monthly spend reports";

// Both of the report's queries answer (name, spend) rows: a team alias or one expanded request tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendRow {
    pub name: Option<String>,
    pub total_spend: Decimal,
}

// Which of _get_daily_spend_reports' two queries a source runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendTotals {
    Teams,
    Tags,
}

pub type SpendReport = (Vec<SpendRow>, Vec<SpendRow>);

#[derive(Debug, Error)]
pub enum ReportError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the database returned an unreadable spend in the {column} column")]
    Spend {
        column: &'static str,
        #[source]
        source: rust_decimal::Error,
    },
}

// _get_spend_report_for_time_range parses the two dates before its try, so a malformed date is not swallowed.
#[derive(Debug, Error)]
pub enum ReportDateError {
    #[error("the {field} date {value:?} is not a YYYY-MM-DD date")]
    Format {
        field: &'static str,
        value: String,
        #[source]
        source: chrono::ParseError,
    },
    #[error("the {field} date {value:?} has no midnight")]
    Midnight { field: &'static str, value: String },
}

// The report's two queries, so the weekly/monthly report owner stays free of the Slack integration.
#[async_trait]
pub trait SpendReportSource: Send + Sync {
    async fn totals(
        &self,
        totals: SpendTotals,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<SpendRow>, ReportError>;
}

pub async fn spend_report_for_time_range(
    source: Option<&dyn SpendReportSource>,
    start_date: &str,
    end_date: &str,
) -> Result<Option<SpendReport>, ReportDateError> {
    let Some(source) = source else {
        tracing::error!("{DATABASE_NOT_CONNECTED}");
        return Ok(None);
    };
    let start = utc_midnight(start_date, "start_date")?;
    let end = utc_midnight(end_date, "end_date")?;
    let teams = match source.totals(SpendTotals::Teams, start, end).await {
        Ok(rows) => rows,
        Err(error) => return logged_failure(error),
    };
    let tags = match source.totals(SpendTotals::Tags, start, end).await {
        Ok(rows) => rows,
        Err(error) => return logged_failure(error),
    };
    Ok(Some((teams, tags)))
}

// the source logs the exception and answers None, as the report caller expects a missing pair
fn logged_failure(error: ReportError) -> Result<Option<SpendReport>, ReportDateError> {
    tracing::error!(error = %error, "Exception in _get_daily_spend_reports");
    Ok(None)
}

// strptime("%Y-%m-%d") with timezone.utc, which Prisma binds as an explicit +00:00 timestamp
fn utc_midnight(value: &str, field: &'static str) -> Result<DateTime<Utc>, ReportDateError> {
    let date =
        NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|source| ReportDateError::Format {
            field,
            value: value.to_owned(),
            source,
        })?;
    let midnight = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| ReportDateError::Midnight {
            field,
            value: value.to_owned(),
        })?;
    Ok(Utc.from_utc_datetime(&midnight))
}
