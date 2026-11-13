use betterlite_models::token_cost::GenericCostResult;

use crate::cost::call_price::Call;
use crate::cost::call_price::error::CallPriceError;
use crate::cost::call_price::policy::CharacterPriceRequest;

pub(super) fn vertex_characters(
    call: &Call<'_>,
    model: &str,
) -> Result<GenericCostResult, CallPriceError> {
    let request = CharacterPriceRequest {
        provider: &call.names.provider,
        model,
        prompt_characters: call.input.prompt_characters,
        completion_characters: call.input.completion_characters,
        usage: &call.view.cost,
        prompt_extras: &call.view.extras,
        service_tier: call.input.service_tier,
        vertex_location: call.input.vertex_location,
    };
    Ok(call.env.policy.character_cost(&request)?)
}
