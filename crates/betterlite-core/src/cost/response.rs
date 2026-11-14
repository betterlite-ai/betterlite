use std::time::SystemTime;

use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;

use crate::cost::completion::args::{CompletionArgs, CostCalculator};
use crate::cost::completion::completion_cost;
use crate::cost::completion::error::{ChargeInputError, CompletionCostError};
use crate::cost::context::ProviderInference;
use crate::cost::image_dispatch::policy::ImagePricePolicy;
use crate::cost::prepare::ResponseView;
use crate::cost::special::policy::ModelResolver;

mod tests;

const COST_HEADER: &str = "llm_provider-x-litellm-response-cost";

// Ports get_response_cost_from_hidden_params: the provider's own cost header, when the response carries a nonnull one.
fn header_cost(response: Option<&ResponseView<'_>>) -> Result<Option<Decimal>, ChargeInputError> {
    let Some(ResponseView::Typed(typed)) = response else {
        return Ok(None);
    };
    let Some(Json::Object(headers)) = typed
        .hidden_params
        .and_then(|hidden| hidden.get("additional_headers"))
    else {
        return Ok(None);
    };
    let text = match headers.get(COST_HEADER) {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Bool(flag)) => return Ok(Some(Decimal::from(u8::from(*flag)))),
        Some(Json::Number(number)) => number.to_string(),
        Some(Json::Text(text)) => text.trim().to_owned(),
        Some(Json::List(_) | Json::Object(_)) => {
            return Err(ChargeInputError::CostHeader(format!(
                "{COST_HEADER} must be a number or a numeric string"
            )));
        }
    };
    parse_decimal_lexeme(&text)
        .map(Some)
        .map_err(|source| ChargeInputError::CostHeader(source.to_string()))
}

// Ports response_cost_calculator: a cache hit is free, a provider-stated cost wins, and anything else is completion_cost.
pub async fn response_cost_calculator<Resolver, ImagePolicy, Inference, Target, Clock>(
    services: &CostCalculator<'_, Resolver, ImagePolicy, Inference>,
    args: &CompletionArgs<'_>,
    cache_hit: Option<bool>,
    clock: &mut Clock,
    target: Option<&mut Target>,
) -> Result<Decimal, CompletionCostError<ImagePolicy::Error>>
where
    Resolver: ModelResolver,
    ImagePolicy: ImagePricePolicy,
    Inference: ProviderInference + Sync,
    Target: CostBreakdownTarget,
    Clock: FnMut() -> SystemTime,
{
    if cache_hit == Some(true) {
        return Ok(Decimal::ZERO);
    }
    if let Some(cost) = header_cost(args.billing.response)? {
        return Ok(cost);
    }
    completion_cost(services, args, clock, target).await
}
