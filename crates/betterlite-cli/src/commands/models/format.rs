use chrono::DateTime;
use rust_decimal::Decimal;

use super::ModelsError;

const PER_THOUSAND: u32 = 1000;
const MINUTE_FORMAT: &str = "%Y-%m-%d %H:%M";

type Formatted = Result<String, ModelsError>;

pub(super) fn format_iso_datetime_str(value: Option<&str>, field: &'static str) -> Formatted {
    let Some(text) = value else {
        return Ok(String::new());
    };
    DateTime::parse_from_rfc3339(text)
        .map(|datetime| datetime.format(MINUTE_FORMAT).to_string())
        .map_err(|source| ModelsError::DateTime { field, source })
}

pub(super) fn format_timestamp(value: Option<i64>, field: &'static str) -> Formatted {
    let Some(seconds) = value else {
        return Ok(String::new());
    };
    let datetime = DateTime::from_timestamp(seconds, 0).ok_or(ModelsError::Timestamp { field })?;
    Ok(datetime.format(MINUTE_FORMAT).to_string())
}

pub(super) fn format_cost_per_1k_tokens(value: Option<Decimal>, field: &'static str) -> Formatted {
    let Some(cost) = value else {
        return Ok(String::new());
    };
    let per_thousand = cost
        .checked_mul(Decimal::from(PER_THOUSAND))
        .ok_or(ModelsError::Cost { field })?;
    Ok(format!("${:.4}", per_thousand.round_dp(4)))
}
