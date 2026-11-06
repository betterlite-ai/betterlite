use jiff::civil;

use super::{Fold, add_days, apply_time_of_day};
use crate::duration::error::DurationError;

pub(super) fn handle_second(
    current: civil::DateTime,
    base_midnight: civil::DateTime,
    amount: u64,
    fold: Fold,
) -> Result<(civil::DateTime, Fold), DurationError> {
    if amount == 0 {
        return Ok((current, fold));
    }
    let aligned = align_to_next(i128::from(current.second()), i128::from(amount))?;
    let carried_minutes = aligned
        .checked_div(60)
        .ok_or(DurationError::CalendarOverflow)?;
    let second_of_minute = aligned
        .checked_rem(60)
        .ok_or(DurationError::CalendarOverflow)?;
    let minute_total = i128::from(current.minute())
        .checked_add(carried_minutes)
        .ok_or(DurationError::CalendarOverflow)?;
    let hour_total = i128::from(current.hour())
        .checked_add(
            minute_total
                .checked_div(60)
                .ok_or(DurationError::CalendarOverflow)?,
        )
        .ok_or(DurationError::CalendarOverflow)?;
    let minute_of_hour = minute_total
        .checked_rem(60)
        .ok_or(DurationError::CalendarOverflow)?;
    let (source, target_fold) = roll_over(current, base_midnight, hour_total, fold)?;
    let civil = replace_time(source, hour_total, minute_of_hour, second_of_minute)?;
    Ok((civil, target_fold))
}

pub(super) fn handle_minute(
    current: civil::DateTime,
    base_midnight: civil::DateTime,
    amount: u64,
    fold: Fold,
) -> Result<(civil::DateTime, Fold), DurationError> {
    if amount == 0 {
        return Ok((current, fold));
    }
    let aligned = align_to_next(i128::from(current.minute()), i128::from(amount))?;
    let hour_total = i128::from(current.hour())
        .checked_add(
            aligned
                .checked_div(60)
                .ok_or(DurationError::CalendarOverflow)?,
        )
        .ok_or(DurationError::CalendarOverflow)?;
    let minute_of_hour = aligned
        .checked_rem(60)
        .ok_or(DurationError::CalendarOverflow)?;
    let (source, target_fold) = roll_over(current, base_midnight, hour_total, fold)?;
    let civil = replace_time(source, hour_total, minute_of_hour, 0)?;
    Ok((civil, target_fold))
}

pub(super) fn handle_hour(
    current: civil::DateTime,
    base_midnight: civil::DateTime,
    amount: u64,
    fold: Fold,
) -> Result<(civil::DateTime, Fold), DurationError> {
    if amount == 0 {
        return Ok((current, fold));
    }
    let aligned = align_to_next(i128::from(current.hour()), i128::from(amount))?;
    let (source, target_fold) = roll_over(current, base_midnight, aligned, fold)?;
    let civil = replace_time(source, aligned, 0, 0)?;
    Ok((civil, target_fold))
}

pub(super) fn handle_day(
    current: civil::DateTime,
    base_midnight: civil::DateTime,
    amount: u64,
    time_of_day: civil::Time,
    fold: Fold,
) -> Result<(civil::DateTime, Fold), DurationError> {
    if amount == 0 {
        return Ok((current, fold));
    }
    if amount == 1 {
        let candidate = apply_time_of_day(base_midnight, time_of_day)?;
        if candidate <= current {
            return Ok((add_days(candidate, 1)?, Fold::Before));
        }
        return Ok((candidate, fold));
    }
    if amount == 7 {
        let weekday = current.weekday().to_monday_zero_offset();
        let until_monday = 7_i8
            .checked_sub(weekday)
            .ok_or(DurationError::CalendarOverflow)?
            .checked_rem(7)
            .ok_or(DurationError::CalendarOverflow)?;
        let upcoming_monday = add_days(base_midnight, i64::from(until_monday))?;
        let candidate = apply_time_of_day(upcoming_monday, time_of_day)?;
        if candidate <= current {
            return Ok((add_days(candidate, 7)?, Fold::Before));
        }
        return Ok((candidate, Fold::Before));
    }
    if amount == 30 {
        return handle_month(current, base_midnight, 1, time_of_day, fold);
    }
    let days =
        i64::try_from(amount).map_err(|error| DurationError::CalendarField { source: error })?;
    let candidate = add_days(base_midnight, days)?;
    Ok((apply_time_of_day(candidate, time_of_day)?, Fold::Before))
}

pub(super) fn handle_month(
    current: civil::DateTime,
    base_midnight: civil::DateTime,
    amount: u64,
    time_of_day: civil::Time,
    fold: Fold,
) -> Result<(civil::DateTime, Fold), DurationError> {
    if amount != 1 {
        return Err(DurationError::UnsupportedMonthInterval { quantity: amount });
    }
    let first_of_month = base_midnight
        .with()
        .day(1)
        .build()
        .map_err(|error| DurationError::Calendar { source: error })?;
    let candidate = apply_time_of_day(first_of_month, time_of_day)?;
    if candidate <= current {
        let next = first_of_next_month(first_of_month)?;
        return Ok((apply_time_of_day(next, time_of_day)?, fold));
    }
    Ok((candidate, fold))
}

fn align_to_next(field: i128, value: i128) -> Result<i128, DurationError> {
    let remainder = field
        .checked_rem(value)
        .ok_or(DurationError::CalendarOverflow)?;
    let raised = field
        .checked_add(value)
        .ok_or(DurationError::CalendarOverflow)?;
    raised
        .checked_sub(remainder)
        .ok_or(DurationError::CalendarOverflow)
}

fn roll_over(
    current: civil::DateTime,
    base_midnight: civil::DateTime,
    hour_total: i128,
    fold: Fold,
) -> Result<(civil::DateTime, Fold), DurationError> {
    if hour_total >= 24 {
        return Ok((add_days(base_midnight, 1)?, Fold::Before));
    }
    Ok((current, fold))
}

fn replace_time(
    source: civil::DateTime,
    hour: i128,
    minute: i128,
    second: i128,
) -> Result<civil::DateTime, DurationError> {
    let hour_field = i8::try_from(
        hour.checked_rem(24)
            .ok_or(DurationError::CalendarOverflow)?,
    )
    .map_err(|error| DurationError::CalendarField { source: error })?;
    let minute_field =
        i8::try_from(minute).map_err(|error| DurationError::CalendarField { source: error })?;
    let second_field =
        i8::try_from(second).map_err(|error| DurationError::CalendarField { source: error })?;
    let time = civil::Time::new(hour_field, minute_field, second_field, 0)
        .map_err(|error| DurationError::Calendar { source: error })?;
    Ok(source.date().to_datetime(time))
}

fn first_of_next_month(first: civil::DateTime) -> Result<civil::DateTime, DurationError> {
    let (year, month) = if first.month() == 12 {
        (
            first
                .year()
                .checked_add(1)
                .ok_or(DurationError::CalendarOverflow)?,
            1,
        )
    } else {
        (
            first.year(),
            first
                .month()
                .checked_add(1)
                .ok_or(DurationError::CalendarOverflow)?,
        )
    };
    civil::Date::new(year, month, 1)
        .map(|date| date.to_datetime(first.time()))
        .map_err(|error| DurationError::Calendar { source: error })
}
