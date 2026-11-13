use betterlite_models::token_cost::GenericCostResult;

use crate::cost::call_price::Call;
use crate::cost::call_price::error::CallPriceError;
use crate::cost::call_price::policy::TokenPriceRequest;

pub(super) fn policy_tokens(
    call: &Call<'_>,
    model: &str,
    service_tier: Option<&str>,
    vertex_location: Option<&str>,
) -> Result<GenericCostResult, CallPriceError> {
    Ok(call.env.policy.token_cost(&TokenPriceRequest {
        provider: &call.names.provider,
        model,
        usage: &call.view.cost,
        prompt_extras: &call.view.extras,
        service_tier,
        vertex_location,
    })?)
}
