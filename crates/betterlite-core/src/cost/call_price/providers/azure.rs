use betterlite_foundry::pricing::response::{ResponseCostRequest, cost_per_token};
use betterlite_models::token_cost::GenericCostResult;

use crate::cost::call_price::Call;
use crate::cost::call_price::error::CallPriceError;

pub(super) fn azure_ai(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    Ok(cost_per_token(
        &ResponseCostRequest {
            model: &call.names.model,
            usage: &call.view.cost,
            prompt_extras: &call.view.extras,
            response_time_ms: call.input.response_time_ms,
            request_model: call.input.request_model,
            service_tier: call.input.service_tier,
            moment: call.env.moment,
        },
        call.env.lookup,
        call.env.database,
    )?)
}
