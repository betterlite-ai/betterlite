use betterlite_models::token_cost::GenericCostResult;

use crate::cost::call_price::Call;
use crate::cost::call_price::error::CallPriceError;
use crate::cost::call_price::policy::SearchPriceRequest;

pub(super) fn search(call: &Call<'_>) -> Result<GenericCostResult, CallPriceError> {
    let queries = call.input.number_of_queries.filter(|count| *count != 0);
    let request = SearchPriceRequest {
        provider: &call.names.provider,
        model: &call.names.model,
        number_of_queries: queries.unwrap_or(1),
        hidden_params: call.input.search_hidden_params,
    };
    let priced = call.env.policy.search_cost(&request)?;
    Ok(priced)
}
