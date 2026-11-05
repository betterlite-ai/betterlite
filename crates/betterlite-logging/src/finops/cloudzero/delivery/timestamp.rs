use chrono::{DateTime, SecondsFormat, Utc};
use jiff::civil;
use jiff::tz::TimeZone;

use super::error::TimestampError;
use crate::finops::cloudzero::records::{CbfRecord, CbfValue};

pub(super) const USAGE_START: &str = "time/usage_start";
pub(super) const NAIVE_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.f";

pub(super) fn parse_timestamp(
    timestamp: &str,
    user_timezone: &TimeZone,
) -> Result<DateTime<Utc>, TimestampError> {
    let parse_error = |source| TimestampError::Parse {
        timestamp: timestamp.to_owned(),
        source,
    };
    let instant = if let Ok(instant) = timestamp.parse::<jiff::Timestamp>() {
        instant
    } else {
        let local = match timestamp.parse::<civil::DateTime>() {
            Ok(local) => local,
            Err(_) => timestamp
                .parse::<civil::Date>()
                .map_err(parse_error)?
                .to_datetime(civil::Time::midnight()),
        };
        user_timezone.to_timestamp(local).map_err(parse_error)?
    };
    DateTime::from_timestamp(
        instant.as_second(),
        instant.subsec_nanosecond().unsigned_abs(),
    )
    .ok_or_else(|| TimestampError::OutOfRange {
        timestamp: timestamp.to_owned(),
    })
}

pub(super) fn usage_start(
    row: &CbfRecord,
    user_timezone: &TimeZone,
) -> Result<Option<DateTime<Utc>>, TimestampError> {
    match row.get(USAGE_START) {
        None | Some(CbfValue::None) => Ok(None),
        Some(CbfValue::Text(text)) if text.is_empty() => Ok(None),
        Some(CbfValue::Text(text)) => parse_timestamp(text, user_timezone).map(Some),
        Some(CbfValue::DateTime(moment)) => parse_timestamp(
            &moment.naive_utc().format(NAIVE_FORMAT).to_string(),
            user_timezone,
        )
        .map(Some),
        Some(CbfValue::Integer(number)) => Err(TimestampError::NotText {
            timestamp: number.to_string(),
        }),
        Some(CbfValue::Money(amount)) => Err(TimestampError::NotText {
            timestamp: amount.to_string(),
        }),
    }
}

pub(super) fn utc_iso(moment: DateTime<Utc>) -> String {
    let precision = if moment.timestamp_subsec_micros() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Micros
    };
    moment.to_rfc3339_opts(precision, true)
}
