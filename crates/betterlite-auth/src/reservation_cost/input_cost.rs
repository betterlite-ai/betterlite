use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::error::ReservationCostError;
use super::{EstimateRequest, EstimateSources, estimates, loaded_costs};

// The cancellation floor a dispatched request has already paid, so a cancelled reservation refunds no lower.
pub async fn estimate_request_input_cost(
    request: &EstimateRequest<'_>,
    sources: &EstimateSources<'_>,
    input_token_counts: Option<&IndexMap<String, usize>>,
) -> Result<Option<Decimal>, ReservationCostError> {
    loaded_costs(request, sources, input_token_counts)
        .await?
        .into_iter()
        .try_fold(None, |best: Option<Decimal>, (counted, info)| {
            estimates::input_cost_for_info(counted, &info).map(|cost| best.max(cost))
        })
}
