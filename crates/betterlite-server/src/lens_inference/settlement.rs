use betterlite_db::lens_repository::database::Database;
use betterlite_db::lens_repository::repository::{LensRepository, LensRepositoryError};
use betterlite_db::lens_repository::update::{DEFAULT_ATTEMPTS, LensUpdateError};
use betterlite_lens::models::job::Lens;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::lens_inference::jobs::replace_job;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum SettlementError {
    #[error("the lens could not be read or written: {0}")]
    Repository(#[source] LensRepositoryError),
    #[error("the settled lens spend is outside the decimal range")]
    Overflow,
}

impl SettlementError {
    fn from_update(error: LensUpdateError<Self>) -> Self {
        match error {
            LensUpdateError::Repository(source) => Self::Repository(source),
            LensUpdateError::Transform(inner) => inner,
        }
    }
}

pub struct SettleRequest<'call> {
    pub lens_id: &'call str,
    pub job_id: &'call str,
    pub month_now: DateTime<Utc>,
    pub estimate: Decimal,
    pub cost: Decimal,
}

// inference.settle: the estimate comes back off and the billed cost goes on, never below zero
pub fn settle_transform(lens: Lens, request: &SettleRequest<'_>) -> Result<Lens, SettlementError> {
    let charged = lens
        .jobs
        .iter()
        .find(|job| job.id == request.job_id)
        .cloned();
    let mut adjusted = lens;
    if adjusted.budget_month == request.month_now.format("%Y-%m").to_string() {
        let spent = adjusted
            .spent
            .checked_sub(request.estimate)
            .and_then(|spent| spent.checked_add(request.cost))
            .ok_or(SettlementError::Overflow)?;
        adjusted.spent = spent.max(Decimal::ZERO);
    }
    let Some(charged) = charged else {
        return Ok(adjusted);
    };
    let cost = charged
        .cost
        .checked_sub(request.estimate)
        .and_then(|cost| cost.checked_add(request.cost))
        .ok_or(SettlementError::Overflow)?;
    let mut settled = charged;
    settled.cost = cost.max(Decimal::ZERO);
    Ok(replace_job(&adjusted, &settled))
}

// settle: a lens another writer has moved on from is left as it is, and a failed read or write is a 500
pub async fn settle<Db>(
    repository: &LensRepository<Db>,
    request: &SettleRequest<'_>,
) -> Result<(), SettlementError>
where
    Db: Database,
{
    repository
        .update(
            request.lens_id,
            |lens| settle_transform(lens, request),
            DEFAULT_ATTEMPTS,
            false,
        )
        .await
        .map_err(SettlementError::from_update)?;
    Ok(())
}
