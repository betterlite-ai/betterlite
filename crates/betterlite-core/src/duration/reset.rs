use chrono::{DateTime, NaiveTime, Timelike as _, Utc};
use jiff::civil;
use jiff::tz::TimeZoneDatabase;

use super::error::{DurationError, DurationWarning};
use super::spec::{self, ResetDurationSpec, ResetUnit};
use super::zone;
use handlers::{handle_day, handle_hour, handle_minute, handle_month, handle_second};

mod handlers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fold {
    Before,
    After,
}

#[derive(Debug, Clone)]
pub struct Reset {
    pub civil: civil::DateTime,
    pub instant: DateTime<Utc>,
    pub warnings: Vec<DurationWarning>,
}

struct Timeline {
    current: civil::DateTime,
    base_midnight: civil::DateTime,
    time_of_day: civil::Time,
    fold: Fold,
}

impl Timeline {
    fn apply(
        &self,
        amount: u64,
        unit: ResetUnit,
    ) -> Result<(civil::DateTime, Fold), DurationError> {
        match unit {
            ResetUnit::Second => handle_second(self.current, self.base_midnight, amount, self.fold),
            ResetUnit::Minute => handle_minute(self.current, self.base_midnight, amount, self.fold),
            ResetUnit::Hour => handle_hour(self.current, self.base_midnight, amount, self.fold),
            ResetUnit::Day => handle_day(
                self.current,
                self.base_midnight,
                amount,
                self.time_of_day,
                self.fold,
            ),
            ResetUnit::Week => {
                let days = amount
                    .checked_mul(7)
                    .ok_or(DurationError::CalendarOverflow)?;
                handle_day(
                    self.current,
                    self.base_midnight,
                    days,
                    self.time_of_day,
                    self.fold,
                )
            }
            ResetUnit::Month => handle_month(
                self.current,
                self.base_midnight,
                amount,
                self.time_of_day,
                self.fold,
            ),
        }
    }
}

pub fn next_standardized_reset_time(
    spec: &str,
    current_time: DateTime<Utc>,
    timezone: Option<&str>,
    reset_time_of_day: NaiveTime,
    database: &TimeZoneDatabase,
) -> Result<Reset, DurationError> {
    let resolved = zone::resolve_zone(timezone, database);
    let (current, fold) = zone::civil_time(current_time, &resolved.zone)?;
    let timeline = Timeline {
        current,
        base_midnight: midnight(current)?,
        time_of_day: reset_time(reset_time_of_day)?,
        fold,
    };
    let mut warnings = resolved.warnings;
    let (civil, reset_fold) = match spec::parse_reset_duration(spec)? {
        ResetDurationSpec::Known {
            amount,
            unit,
            warnings: spec_warnings,
        } => {
            warnings.extend(spec_warnings);
            timeline.apply(amount, unit)?
        }
        ResetDurationSpec::Unrecognized {
            warnings: spec_warnings,
        }
        | ResetDurationSpec::UnknownUnit {
            warnings: spec_warnings,
            ..
        } => {
            warnings.extend(spec_warnings);
            (add_days(timeline.base_midnight, 1)?, Fold::Before)
        }
    };
    let instant = zone::instant(civil, &resolved.zone, reset_fold)?;
    Ok(Reset {
        civil,
        instant,
        warnings,
    })
}

fn midnight(current: civil::DateTime) -> Result<civil::DateTime, DurationError> {
    current
        .with()
        .hour(0)
        .minute(0)
        .second(0)
        .subsec_nanosecond(0)
        .build()
        .map_err(|error| DurationError::Calendar { source: error })
}

fn apply_time_of_day(
    current: civil::DateTime,
    time_of_day: civil::Time,
) -> Result<civil::DateTime, DurationError> {
    current
        .with()
        .hour(time_of_day.hour())
        .minute(time_of_day.minute())
        .second(time_of_day.second())
        .subsec_nanosecond(time_of_day.subsec_nanosecond())
        .build()
        .map_err(|error| DurationError::Calendar { source: error })
}

fn add_days(moment: civil::DateTime, days: i64) -> Result<civil::DateTime, DurationError> {
    moment
        .checked_add(jiff::Span::new().days(days))
        .map_err(|error| DurationError::Calendar { source: error })
}

fn reset_time(time: NaiveTime) -> Result<civil::Time, DurationError> {
    let hour = i8::try_from(time.hour())
        .map_err(|error| DurationError::CalendarField { source: error })?;
    let minute = i8::try_from(time.minute())
        .map_err(|error| DurationError::CalendarField { source: error })?;
    let second = i8::try_from(time.second())
        .map_err(|error| DurationError::CalendarField { source: error })?;
    let nanos = i32::try_from(time.nanosecond())
        .map_err(|error| DurationError::CalendarField { source: error })?;
    civil::Time::new(hour, minute, second, nanos)
        .map_err(|error| DurationError::Calendar { source: error })
}
