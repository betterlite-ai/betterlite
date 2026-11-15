use std::collections::BTreeMap;

use betterlite_foundry::pricing::additional::additional_costs;
use betterlite_foundry::pricing::names::classify_router_name;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use rust_decimal::Decimal;

use super::inputs::Attempt;
use crate::cost::completion::error::ChargeInputError;
use crate::cost::prepare::ResponseView;

// hidden_params.get("model") or hidden_params.get("litellm_model_name"): the first nonempty text, a wrong type is an error.
fn hidden_model<'hidden>(
    response: Option<&'hidden ResponseView<'hidden>>,
) -> Result<Option<&'hidden str>, ChargeInputError> {
    let Some(ResponseView::Typed(typed)) = response else {
        return Ok(None);
    };
    let Some(hidden) = typed.hidden_params else {
        return Ok(None);
    };
    for field in ["model", "litellm_model_name"] {
        match hidden.get(field) {
            None | Some(Json::Null) => {}
            Some(Json::Text(text)) if text.is_empty() => {}
            Some(Json::Text(text)) => return Ok(Some(text)),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(ChargeInputError::Hidden { field });
            }
        }
    }
    Ok(None)
}

// The provider's own extra fees apply to azure_ai calls whose priced model is not already router-shaped. The hidden model stands in for the request model when it is router-shaped or the request model is missing.
pub(super) fn additional_fees(
    attempt: &Attempt<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Option<BTreeMap<String, Decimal>>, ChargeInputError> {
    if attempt.provider != Some("azure_ai")
        || classify_router_name(attempt.model).is_router_shaped()
    {
        return Ok(None);
    }
    let hidden = hidden_model(attempt.args.billing.response)?;
    let mut named = attempt.request_model_for_cost;
    if hidden.is_some_and(|name| classify_router_name(name).is_router_shaped()) || named.is_none() {
        named = hidden;
    }
    Ok(additional_costs(
        named.unwrap_or(attempt.model),
        attempt.provider,
        attempt.billing.prompt_tokens,
        attempt.billing.completion_tokens,
        lookup,
    ))
}

// The charge before adjustments: both token prices, the built-in tool fees and every additional fee, summed without losing a digit.
pub(super) fn checked_total(
    prompt: Decimal,
    completion: Decimal,
    tools: Decimal,
    additional: Option<&BTreeMap<String, Decimal>>,
) -> Result<Decimal, ChargeInputError> {
    additional
        .into_iter()
        .flat_map(BTreeMap::values)
        .try_fold(Decimal::ZERO, |sum, cost| sum.checked_add(*cost))
        .and_then(|extra| {
            prompt
                .checked_add(completion)?
                .checked_add(tools)?
                .checked_add(extra)
        })
        .ok_or(ChargeInputError::Overflow)
}
