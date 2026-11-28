use betterlite_db::lens_repository::database::Database;
use betterlite_db::lens_repository::repository::{LensRepository, LensRepositoryError};
use betterlite_db::lens_repository::update::{DEFAULT_ATTEMPTS, LensUpdateError};
use betterlite_lens::models::job::Lens;
use betterlite_lens::state::transitions::renew_budget;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::lens_inference::jobs::{current_job, replace_job};

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum ReservationError {
    #[error("Job was cancelled or reassigned")]
    JobUnavailable,
    #[error("Monthly lens budget reached; increase it or wait for next month")]
    BudgetReached,
    #[error("Could not reserve analysis budget")]
    NotReserved,
    #[error("the lens could not be read or written: {0}")]
    Repository(#[source] LensRepositoryError),
    #[error("the lens budget reserve is outside the decimal range")]
    Overflow,
}

impl ReservationError {
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::JobUnavailable | Self::NotReserved => 409,
            Self::BudgetReached => 402,
            Self::Repository(_) | Self::Overflow => 500,
        }
    }

    fn from_update(error: LensUpdateError<Self>) -> Self {
        match error {
            LensUpdateError::Repository(source) => Self::Repository(source),
            LensUpdateError::Transform(inner) => inner,
        }
    }
}

pub struct ReserveRequest<'call> {
    pub lens_id: &'call str,
    pub job_id: &'call str,
    pub worker_id: &'call str,
    pub month_now: DateTime<Utc>,
    pub lease_now: DateTime<Utc>,
    pub estimate: Decimal,
}

// inference.reserve: the analyst's own lease still holds the job and the month has room for the estimate
pub fn reserve_transform(
    lens: &Lens,
    request: &ReserveRequest<'_>,
) -> Result<Lens, ReservationError> {
    let current = renew_budget(lens, request.month_now);
    let Some(active) = current_job(&current) else {
        return Err(ReservationError::JobUnavailable);
    };
    let held = active.id == request.job_id
        && active.worker_id.as_deref() == Some(request.worker_id)
        && active
            .lease_until
            .is_some_and(|lease| lease > request.lease_now);
    if !held {
        return Err(ReservationError::JobUnavailable);
    }
    let spent = current
        .spent
        .checked_add(request.estimate)
        .ok_or(ReservationError::Overflow)?;
    if spent > current.settings.monthly_budget {
        return Err(ReservationError::BudgetReached);
    }
    let mut charged = active.clone();
    charged.cost = charged
        .cost
        .checked_add(request.estimate)
        .ok_or(ReservationError::Overflow)?;
    let mut updated = replace_job(&current, &charged);
    updated.spent = spent;
    Ok(updated)
}

// reserve_budget: a lens that could not be written is a 409, and every other failure carries its own status
pub async fn reserve<Db>(
    repository: &LensRepository<Db>,
    request: &ReserveRequest<'_>,
) -> Result<(), ReservationError>
where
    Db: Database,
{
    let updated = repository
        .update(
            request.lens_id,
            |lens| reserve_transform(&lens, request),
            DEFAULT_ATTEMPTS,
            false,
        )
        .await
        .map_err(ReservationError::from_update)?;
    if updated.is_none() {
        return Err(ReservationError::NotReserved);
    }
    Ok(())
}
