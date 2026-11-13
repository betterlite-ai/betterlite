use betterlite_models::anthropic_pricing::{
    AnthropicPriceRequest, BedrockPriceRequest, TokenPriceRequest, token_cost,
};
use betterlite_models::token_cost::GenericCostResult;

use crate::cost::call_price::Call;
use crate::cost::call_price::error::CallPriceError;

pub(super) fn anthropic_family(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let projection = call
        .input
        .server_tool_projection
        .cloned()
        .unwrap_or_default();
    let request = if call.names.provider == "anthropic" {
        TokenPriceRequest::Anthropic(AnthropicPriceRequest {
            model: &call.names.model,
            usage: &call.view.cost,
            prompt_extras: &call.view.extras,
            supplied_model_info: None,
            provider_specific_entry: None,
            service_tier: call.input.service_tier,
            projection: &projection,
            moment: call.env.moment,
        })
    } else {
        TokenPriceRequest::Bedrock(BedrockPriceRequest {
            model: &call.names.model,
            usage: &call.view.cost,
            prompt_extras: &call.view.extras,
            service_tier: call.input.service_tier,
            moment: call.env.moment,
        })
    };
    Ok(token_cost(&request, call.env.lookup, call.env.database)?)
}
