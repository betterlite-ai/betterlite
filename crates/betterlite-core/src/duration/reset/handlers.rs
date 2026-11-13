use jiff::civil;

use super::{Fold, Timeline, add_days, apply_time_of_day};
use crate::duration::error::DurationError;

#[derive(Clone, Copy)]
pub(super) enum ClockField {
    Hour,
    Minute,
    Second,
}

struct ClockTarget {
    hour: i128,
    minute: i128,
    second: i128,
}

pub(super) fn handle_clock(
    timeline: &Timeline,
    amount: u64,
    field: ClockField,
) -> Result<(civil::DateTime, Fold), DurationError> {
    if amount == 0 {
        return Ok((timeline.current, timeline.fold));
    }
    let target = clock_target(timeline.current, i128::from(amount), field)?;
    let (source, target_fold) = roll_over(timeline, target.hour)?;
    let civil = replace_time(source, target.hour, target.minute, target.second)?;
    Ok((civil, target_fold))
}

pub(super) fn handle_day(
    timeline: &Timeline,
    amount: u64,
) -> Result<(civil::DateTime, Fold), DurationError> {
    if amount == 0 {
        return Ok((timeline.current, timeline.fold));
    }
    if amount == 1 {
        let candidate = apply_time_of_day(timeline.base_midnight, timeline.time_of_day)?;
        if candidate <= timeline.current {
            return Ok((add_days(candidate, 1)?, Fold::Before));
        }
        return Ok((candidate, timeline.fold));
    }
    if amount == 7 {
        let weekday = timeline.current.weekday().to_monday_zero_offset();
        let until_monday = 7_i8
            .checked_sub(weekday)
            .ok_or(DurationError::CalendarOverflow)?
            .checked_rem(7)
            .ok_or(DurationError::CalendarOverflow)?;
        let upcoming_monday = add_days(timeline.base_midnight, i64::from(until_monday))?;
        let candidate = apply_time_of_day(upcoming_monday, timeline.time_of_day)?;
        if candidate <= timeline.current {
            return Ok((add_days(candidate, 7)?, Fold::Before));
        }
        return Ok((candidate, Fold::Before));
    }
    if amount == 30 {
        return handle_month(timeline, 1);
    }
    let days =
        i64::try_from(amount).map_err(|error| DurationError::CalendarField { source: error })?;
    let candidate = add_days(timeline.base_midnight, days)?;
    Ok((
        apply_time_of_day(candidate, timeline.time_of_day)?,
        Fold::Before,
    ))
}

pub(super) fn handle_month(
    timeline: &Timeline,
    amount: u64,
) -> Result<(civil::DateTime, Fold), DurationError> {
    if amount != 1 {
        return Err(DurationError::UnsupportedMonthInterval { quantity: amount });
    }
    let first_of_month = timeline
        .base_midnight
        .with()
        .day(1)
        .build()
        .map_err(|error| DurationError::Calendar { source: error })?;
    let candidate = apply_time_of_day(first_of_month, timeline.time_of_day)?;
    if candidate > timeline.current {
        return Ok((candidate, timeline.fold));
    }
    let next = first_of_next_month(first_of_month)?;
    Ok((
        apply_time_of_day(next, timeline.time_of_day)?,
        timeline.fold,
    ))
}

fn clock_target(
    current: civil::DateTime,
    value: i128,
    field: ClockField,
) -> Result<ClockTarget, DurationError> {
    match field {
        ClockField::Hour => hour_target(current, value),
        ClockField::Minute => minute_target(current, value),
        ClockField::Second => second_target(current, value),
    }
}

fn hour_target(current: civil::DateTime, value: i128) -> Result<ClockTarget, DurationError> {
    Ok(ClockTarget {
        hour: align_to_next(i128::from(current.hour()), value)?,
        minute: 0,
        second: 0,
    })
}

fn minute_target(current: civil::DateTime, value: i128) -> Result<ClockTarget, DurationError> {
    let aligned = align_to_next(i128::from(current.minute()), value)?;
    let carried_hours = aligned
        .checked_div(60)
        .ok_or(DurationError::CalendarOverflow)?;
    let hour = i128::from(current.hour())
        .checked_add(carried_hours)
        .ok_or(DurationError::CalendarOverflow)?;
    let minute = aligned
        .checked_rem(60)
        .ok_or(DurationError::CalendarOverflow)?;
    Ok(ClockTarget {
        hour,
        minute,
        second: 0,
    })
}

fn second_target(current: civil::DateTime, value: i128) -> Result<ClockTarget, DurationError> {
    let aligned = align_to_next(i128::from(current.second()), value)?;
    let total = seconds_since_midnight(current)?
        .checked_add(aligned)
        .ok_or(DurationError::CalendarOverflow)?;
    clock_from_total(total)
}

fn seconds_since_midnight(current: civil::DateTime) -> Result<i128, DurationError> {
    i128::from(current.hour())
        .checked_mul(3_600)
        .and_then(|hours| hours.checked_add(i128::from(current.minute()).checked_mul(60)?))
        .ok_or(DurationError::CalendarOverflow)
}

fn clock_from_total(total: i128) -> Result<ClockTarget, DurationError> {
    let seconds_in_minute = total
        .checked_rem(60)
        .ok_or(DurationError::CalendarOverflow)?;
    let rest_of_hour = total
        .checked_rem(3_600)
        .ok_or(DurationError::CalendarOverflow)?;
    Ok(ClockTarget {
        hour: total
            .checked_div(3_600)
            .ok_or(DurationError::CalendarOverflow)?,
        minute: rest_of_hour
            .checked_div(60)
            .ok_or(DurationError::CalendarOverflow)?,
        second: seconds_in_minute,
    })
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
    timeline: &Timeline,
    hour_total: i128,
) -> Result<(civil::DateTime, Fold), DurationError> {
    if hour_total >= 24 {
        return Ok((add_days(timeline.base_midnight, 1)?, Fold::Before));
    }
    Ok((timeline.current, timeline.fold))
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
