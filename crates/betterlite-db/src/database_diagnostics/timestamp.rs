use chrono::{DateTime, FixedOffset, NaiveDateTime, Utc};
use rust_decimal::Decimal;

pub trait LocalTime {
    fn local(&self, instant: DateTime<Utc>) -> NaiveDateTime;
}

pub struct SystemLocalTime;

impl LocalTime for SystemLocalTime {
    fn local(&self, instant: DateTime<Utc>) -> NaiveDateTime {
        instant.with_timezone(&chrono::Local).naive_local()
    }
}

pub struct FixedLocalTime {
    pub offset: FixedOffset,
}

impl LocalTime for FixedLocalTime {
    fn local(&self, instant: DateTime<Utc>) -> NaiveDateTime {
        instant.with_timezone(&self.offset).naive_local()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TimestampError {
    #[error("the log record's timestamp cannot be represented")]
    Unrepresentable,
}

pub fn format_time(
    local_time: &dyn LocalTime,
    created: f64,
    _datefmt: Option<&str>,
) -> Result<String, TimestampError> {
    let instant = instant_from_seconds(created)?;
    let local = local_time.local(instant);
    Ok(iso_timestamp(local))
}

pub fn legacy_time(local_time: &dyn LocalTime, created: f64) -> Result<String, TimestampError> {
    let seconds = Decimal::from_f64_retain(created).ok_or(TimestampError::Unrepresentable)?;
    let whole = seconds.trunc();
    let remainder = seconds
        .checked_sub(whole)
        .ok_or(TimestampError::Unrepresentable)?;
    let millis_scaled = remainder
        .checked_mul(Decimal::from(1000_i64))
        .ok_or(TimestampError::Unrepresentable)?;
    let millis =
        i64::try_from(millis_scaled.trunc()).map_err(|_source| TimestampError::Unrepresentable)?;
    let whole_seconds = i64::try_from(whole).map_err(|_source| TimestampError::Unrepresentable)?;
    let instant =
        DateTime::from_timestamp(whole_seconds, 0).ok_or(TimestampError::Unrepresentable)?;
    let local = local_time.local(instant);
    Ok(format!("{},{millis:03}", local.format("%Y-%m-%d %H:%M:%S")))
}

fn instant_from_seconds(created: f64) -> Result<DateTime<Utc>, TimestampError> {
    let seconds = Decimal::from_f64_retain(created).ok_or(TimestampError::Unrepresentable)?;
    let scaled = seconds
        .checked_mul(Decimal::from(1_000_000_i64))
        .ok_or(TimestampError::Unrepresentable)?;
    let micros =
        i64::try_from(scaled.round()).map_err(|_source| TimestampError::Unrepresentable)?;
    DateTime::from_timestamp_micros(micros).ok_or(TimestampError::Unrepresentable)
}

fn iso_timestamp(local: NaiveDateTime) -> String {
    let base = local.format("%Y-%m-%dT%H:%M:%S").to_string();
    let micros = local.and_utc().timestamp_subsec_micros();
    if micros == 0 {
        base
    } else {
        format!("{base}.{micros:06}")
    }
}
