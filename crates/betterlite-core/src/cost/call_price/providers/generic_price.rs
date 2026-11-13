use betterlite_models::token_cost::{
    GenericCostRequest, GenericCostResult, generic_cost_per_token,
};

use crate::cost::call_price::Call;
use crate::cost::call_price::error::CallPriceError;

pub fn generic(
    call: &Call<'_>,
    model: &str,
    data_residency: Option<&str>,
) -> Result<GenericCostResult, CallPriceError> {
    Ok(generic_cost_per_token(
        &GenericCostRequest {
            model,
            custom_llm_provider: Some(&call.names.provider),
            usage: &call.view.cost,
            prompt_extras: &call.view.extras,
            supplied_pricing: None,
            service_tier: call.input.service_tier,
            data_residency,
            vertex_location: None,
            moment: call.env.moment,
        },
        call.env.lookup,
        call.env.database,
    )?)
}
