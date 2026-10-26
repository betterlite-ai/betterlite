use chrono::{Days, NaiveDate, NaiveTime};
use rust_decimal::{Decimal, RoundingStrategy};

use super::schema::{DECIMAL_SCALE, FOCUS_NORMALIZED_SCHEMA};
use super::table::{ColumnValues, FocusColumn, FocusTable};

mod projection;
mod tags;

const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageRow {
    pub date: String,
    pub api_key: String,
    pub spend: Decimal,
    pub api_requests: i64,
    pub user_id: Option<String>,
    pub model: Option<String>,
    pub model_group: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub organization_id: Option<String>,
    pub organization_alias: Option<String>,
    pub user_email: Option<String>,
    pub api_key_alias: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TransformError {
    #[error("{field} value {value} does not fit Decimal(18, 6)")]
    Overflow { field: &'static str, value: Decimal },
}

struct Charge<'row> {
    row: &'row UsageRow,
    cost: Decimal,
    quantity: Decimal,
    period_start: Option<String>,
    period_end: Option<String>,
    tags: String,
}

pub fn transform(rows: &[UsageRow]) -> Result<FocusTable, TransformError> {
    if rows.is_empty() {
        let columns = FOCUS_NORMALIZED_SCHEMA
            .iter()
            .map(|&(name, kind)| FocusColumn {
                name,
                values: ColumnValues::empty(kind),
            })
            .collect();
        return Ok(FocusTable { rows: 0, columns });
    }
    let charges = rows.iter().map(charge).collect::<Result<Vec<_>, _>>()?;
    let columns = projection::COLUMNS
        .iter()
        .map(|&(name, source)| FocusColumn {
            name,
            values: source.project(&charges),
        })
        .collect();
    Ok(FocusTable {
        rows: rows.len(),
        columns,
    })
}

fn charge(row: &UsageRow) -> Result<Charge<'_>, TransformError> {
    // LiteLLM parses the date with strict=False, so a date it cannot read becomes a null period
    let start = NaiveDate::parse_from_str(&row.date, "%Y-%m-%d")
        .ok()
        .map(|date| date.and_time(NaiveTime::MIN));
    let end = start.and_then(|time| time.checked_add_days(Days::new(1)));
    Ok(Charge {
        row,
        cost: fit(row.spend, "spend")?,
        quantity: fit(Decimal::from(row.api_requests), "api_requests")?,
        period_start: start.map(|time| time.format(TIMESTAMP_FORMAT).to_string()),
        period_end: end.map(|time| time.format(TIMESTAMP_FORMAT).to_string()),
        tags: tags::json(row),
    })
}

fn fit(value: Decimal, field: &'static str) -> Result<Decimal, TransformError> {
    let rounded = value.round_dp_with_strategy(
        u32::from(DECIMAL_SCALE),
        RoundingStrategy::MidpointNearestEven,
    );
    // Decimal(18, 6) leaves twelve digits before the point
    if rounded.abs() < Decimal::new(1_000_000_000_000, 0) {
        Ok(rounded)
    } else {
        Err(TransformError::Overflow { field, value })
    }
}

mod tests;
