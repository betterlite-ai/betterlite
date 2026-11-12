use betterlite_models::duration_cost::openai_seconds::cost_per_second;
use betterlite_models::token_cost::GenericCostResult;

use crate::cost::call_price::Call;
use crate::cost::call_price::error::CallPriceError;
use crate::cost::call_price::providers::generic_price::generic;

// Positive token evidence bills tokens; otherwise the audio duration bills.
pub(super) fn transcription(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let input = call.input;
    let provider = call.names.provider.as_str();
    let model = call.names.without_prefix.as_str();
    let usage = &call.view.cost;
    let details_have_tokens = usage.prompt_details.is_some_and(|details| {
        details.audio_tokens.unwrap_or(0) > 0 || details.text_tokens.unwrap_or(0) > 0
    });
    if details_have_tokens || usage.prompt_tokens > 0 || usage.completion_tokens > 0 {
        return generic(call, model, input.data_residency);
    }
    let (prompt_cost, completion_cost) =
        cost_per_second(model, Some(provider), input.audio_seconds, call.env.lookup)?;
    Ok(GenericCostResult {
        prompt_cost,
        completion_cost,
    })
}
