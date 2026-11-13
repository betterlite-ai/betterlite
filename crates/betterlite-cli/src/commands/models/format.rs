use chrono::DateTime;
use rust_decimal::Decimal;

use super::ModelsError;

const PER_THOUSAND: u32 = 1000;
const MINUTE_FORMAT: &str = "%Y-%m-%d %H:%M";

pub(super) fn format_iso_datetime_str(
    value: Option<&str>,
    field: &'static str,
) -> Result<String, ModelsError> {
    let Some(text) = value else {
        return Ok(String::new());
    };
    match DateTime::parse_from_rfc3339(text) {
        Ok(datetime) => Ok(datetime.format(MINUTE_FORMAT).to_string()),
        Err(source) => Err(ModelsError::DateTime { field, source }),
    }
}

pub(super) fn format_timestamp(
    value: Option<i64>,
    field: &'static str,
) -> Result<String, ModelsError> {
    let Some(seconds) = value else {
        return Ok(String::new());
    };
    match DateTime::from_timestamp(seconds, 0) {
        Some(datetime) => Ok(datetime.format(MINUTE_FORMAT).to_string()),
        None => Err(ModelsError::Timestamp { field }),
    }
}

pub(super) fn format_cost_per_1k_tokens(
    value: Option<Decimal>,
    field: &'static str,
) -> Result<String, ModelsError> {
    let Some(cost) = value else {
        return Ok(String::new());
    };
    let per_thousand = cost
        .checked_mul(Decimal::from(PER_THOUSAND))
        .ok_or(ModelsError::Cost { field })?;
    Ok(format!("${:.4}", per_thousand.round_dp(4)))
}
