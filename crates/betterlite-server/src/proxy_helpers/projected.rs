use chrono::{Datelike as _, Days, Months, NaiveDate};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedLimit {
    pub projected_spend: Decimal,
    pub limit_exceed_date: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    #[error("the projected spend arithmetic overflowed")]
    Arithmetic,
    #[error("the projected limit date is {days} days away, past the calendar chrono carries")]
    DateRange { days: Decimal },
}

#[must_use]
pub fn month_end_date(today: NaiveDate) -> NaiveDate {
    let last_day = today
        .with_day(1)
        .and_then(|first| first.checked_add_months(Months::new(1)))
        .and_then(|first| first.pred_opt());
    // the server's clock never reaches chrono's last month; MAX is the only total answer if it did
    last_day.unwrap_or(NaiveDate::MAX)
}

fn daily_spend(spend: Decimal, today: NaiveDate) -> Result<Decimal, ProjectionError> {
    if today.day() == 1 {
        return Ok(spend);
    }
    spend
        .checked_div(Decimal::from(today.day().saturating_sub(1)))
        .ok_or(ProjectionError::Arithmetic)
}

fn projected_spend(spend: Decimal, today: NaiveDate) -> Result<Decimal, ProjectionError> {
    let remaining_days = Decimal::from(
        month_end_date(today)
            .signed_duration_since(today)
            .num_days(),
    );
    let growth = daily_spend(spend, today)?
        .checked_mul(remaining_days)
        .ok_or(ProjectionError::Arithmetic)?;
    spend.checked_add(growth).ok_or(ProjectionError::Arithmetic)
}

pub fn is_projected_spend_over_limit(
    spend: Decimal,
    soft_limit: Option<Decimal>,
    today: NaiveDate,
) -> Result<bool, ProjectionError> {
    let Some(limit) = soft_limit else {
        return Ok(false);
    };
    Ok(projected_spend(spend, today)? > limit)
}

pub fn projected_spend_over_limit(
    spend: Decimal,
    soft_limit: Option<Decimal>,
    today: NaiveDate,
) -> Result<Option<ProjectedLimit>, ProjectionError> {
    let Some(limit) = soft_limit else {
        return Ok(None);
    };
    let projected = projected_spend(spend, today)?;
    if projected <= limit {
        return Ok(None);
    }
    let daily = daily_spend(spend, today)?;
    if daily <= Decimal::ZERO {
        return Ok(Some(ProjectedLimit {
            projected_spend: projected,
            limit_exceed_date: today,
        }));
    }
    let remaining_budget = limit
        .checked_sub(spend)
        .ok_or(ProjectionError::Arithmetic)?;
    if remaining_budget <= Decimal::ZERO {
        return Ok(Some(ProjectedLimit {
            projected_spend: projected,
            limit_exceed_date: today,
        }));
    }
    let approx_days = remaining_budget
        .checked_div(daily)
        .ok_or(ProjectionError::Arithmetic)?;
    let signed_days = approx_days
        .floor()
        .to_i64()
        .ok_or(ProjectionError::DateRange { days: approx_days })?;
    let whole_days = u64::try_from(signed_days)
        .map_err(|_source| ProjectionError::DateRange { days: approx_days })?;
    let date = today
        .checked_add_days(Days::new(whole_days))
        .ok_or(ProjectionError::DateRange { days: approx_days })?;
    Ok(Some(ProjectedLimit {
        projected_spend: projected,
        limit_exceed_date: date,
    }))
}
