use chrono::{DateTime, Days, NaiveDate, NaiveTime, Utc};
use rust_decimal::Decimal;

const MICROS_PER_HOUR: i64 = 3_600_000_000;

// Valid PTU terms arrive through ptu_terms; a caller can still hand a PtuModel back in, so the window is read as given.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtuModel {
    pub model_id: String,
    pub model_name: String,
    pub team_id: String,
    pub ptu_count: u32,
    pub cost_per_ptu_per_hour: Decimal,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtuCharge {
    pub team_id: String,
    pub model_id: String,
    pub model_name: String,
    pub flat_cost: Decimal,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChargeError {
    #[error("the UTC day after {day} does not exist, so that day's PTU window cannot be bounded")]
    DayOverflow { day: NaiveDate },
    #[error(
        "the flat cost for team '{team_id}' model '{model_id}' on {day} does not fit the \
         Decimal range"
    )]
    AmountOverflow {
        team_id: String,
        model_id: String,
        day: NaiveDate,
    },
}

pub fn active_hours_on_day(model: &PtuModel, day: NaiveDate) -> Result<Decimal, ChargeError> {
    let (day_start, day_end) = day_bounds(day)?;
    let start = model.effective_from.max(day_start);
    let end = model
        .effective_to
        .map_or(day_end, |close| close.min(day_end));
    if end <= start {
        return Ok(Decimal::ZERO);
    }
    let overflow = || ChargeError::AmountOverflow {
        team_id: model.team_id.clone(),
        model_id: model.model_id.clone(),
        day,
    };
    let overlap = end.signed_duration_since(start);
    let micros = overlap.num_microseconds().ok_or_else(overflow)?;
    Decimal::from(micros)
        .checked_div(Decimal::from(MICROS_PER_HOUR))
        .ok_or_else(overflow)
}

pub fn daily_flat_cost(model: &PtuModel, day: NaiveDate) -> Result<Decimal, ChargeError> {
    let hours = active_hours_on_day(model, day)?;
    Decimal::from(model.ptu_count)
        .checked_mul(model.cost_per_ptu_per_hour)
        .and_then(|amount| amount.checked_mul(hours))
        .ok_or_else(|| ChargeError::AmountOverflow {
            team_id: model.team_id.clone(),
            model_id: model.model_id.clone(),
            day,
        })
}

#[must_use]
pub fn aggregate_charges(
    models: &[PtuModel],
    day: NaiveDate,
) -> Result<Vec<PtuCharge>, ChargeError> {
    let mut ordered: Vec<&PtuModel> = models.iter().collect();
    ordered.sort_by(|left, right| {
        (left.team_id.as_str(), left.model_id.as_str())
            .cmp(&(right.team_id.as_str(), right.model_id.as_str()))
    });
    let mut charges = Vec::new();
    for model in ordered {
        let flat_cost = daily_flat_cost(model, day)?;
        if flat_cost > Decimal::ZERO {
            charges.push(PtuCharge {
                team_id: model.team_id.clone(),
                model_id: model.model_id.clone(),
                model_name: model.model_name.clone(),
                flat_cost,
            });
        }
    }
    Ok(charges)
}

fn day_bounds(day: NaiveDate) -> Result<(DateTime<Utc>, DateTime<Utc>), ChargeError> {
    let next = day
        .checked_add_days(Days::new(1))
        .ok_or(ChargeError::DayOverflow { day })?;
    Ok((
        day.and_time(NaiveTime::MIN).and_utc(),
        next.and_time(NaiveTime::MIN).and_utc(),
    ))
}
