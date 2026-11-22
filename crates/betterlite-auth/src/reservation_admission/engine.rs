use rust_decimal::Decimal;

use crate::budget_counter::BudgetCounter;
use crate::reservation_settlement::entry::ReservationEntry;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::counters::AdmissionCounters;
use super::error::AdmissionError;
use super::initialize::initialize_counters;
use super::reserve::reserve_reservable_counters;

// the counters a request reserves against and the policy settings the reservation applies
pub struct AdmissionRequest<'a> {
    pub counters: &'a [BudgetCounter],
    pub token: Option<&'a UserAPIKeyAuth>,
    pub reservation_cost: Decimal,
    pub fail_closed_budget_enforcement: bool,
    // litellm.budget_exceeded_status_code, 422 unless the caller supplies the configured value
    pub budget_exceeded_status: u16,
    // litellm.budget_exceeded_throttle_percentage
    pub throttle_percentage: Option<f64>,
}

// the cost the reservation still holds once the over-budget policy has settled, which the caller settles against the actual cost
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Admission {
    pub reserved_cost: Decimal,
}

// reserve_budget_for_request's reservation step: initialize the counters, then charge them group by group through the over-budget policy
pub async fn admit<C>(
    counters: &C,
    request: &AdmissionRequest<'_>,
    applied_entries: &mut Vec<ReservationEntry>,
) -> Result<Admission, AdmissionError>
where
    C: AdmissionCounters,
{
    let reservable = initialize_counters(
        counters,
        request.counters,
        request.fail_closed_budget_enforcement,
    )
    .await?;
    let reserved_cost =
        reserve_reservable_counters(counters, request, &reservable, applied_entries).await?;
    Ok(Admission { reserved_cost })
}
