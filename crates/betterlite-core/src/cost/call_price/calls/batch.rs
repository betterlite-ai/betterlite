use betterlite_models::token_cost::GenericCostResult;

use crate::cost::batch::{BatchCostRequest, batch_cost_per_token};
use crate::cost::call_price::Call;
use crate::cost::call_price::error::CallPriceError;

pub(super) fn batch(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let request = BatchCostRequest {
        model: &call.names.model,
        custom_llm_provider: Some(&call.names.provider),
        usage: &call.view.cost,
        prompt_extras: &call.view.extras,
        supplied_model_info: None,
        data_residency: call.input.data_residency,
    };
    batch_cost_per_token(&request, call.env.lookup, call.env.resolution)
        .map(|priced| GenericCostResult {
            prompt_cost: priced.prompt_cost,
            completion_cost: priced.completion_cost,
        })
        .map_err(CallPriceError::from)
}
