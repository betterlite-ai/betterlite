use std::borrow::Cow;
use std::str::FromStr as _;

use betterlite_logging::fields::usage::Usage;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::BillingUsageError;

const RESPONSE_COST_HEADER: &str = "llm_provider-x-litellm-response-cost";

#[must_use]
pub fn without_provider_stated_cost(usage: &Usage) -> Cow<'_, Usage> {
    if usage.cost.is_none() {
        return Cow::Borrowed(usage);
    }
    let mut cleared = usage.clone();
    cleared.cost = None;
    Cow::Owned(cleared)
}

pub fn hidden_response_cost(
    hidden_params: Option<&IndexMap<String, Json>>,
) -> Result<Option<Decimal>, BillingUsageError> {
    let Some(hidden) = hidden_params else {
        return Ok(None);
    };
    let Some(headers) = hidden.get("additional_headers") else {
        return Ok(None);
    };
    let Json::Object(header_map) = headers else {
        return Err(BillingUsageError::HiddenParameter {
            field: "additional_headers",
        });
    };
    let Some(cost) = header_map.get(RESPONSE_COST_HEADER) else {
        return Ok(None);
    };
    match cost {
        Json::Null => Ok(None),
        Json::Number(number) => decimal_text(&number.to_string())
            .map(Some)
            .ok_or(BillingUsageError::ResponseCostNotNumber),
        Json::Text(text) => decimal_text(text)
            .map(Some)
            .ok_or(BillingUsageError::ResponseCostNotNumber),
        Json::Bool(_) | Json::List(_) | Json::Object(_) => {
            Err(BillingUsageError::ResponseCostNotNumber)
        }
    }
}

fn decimal_text(text: &str) -> Option<Decimal> {
    let parsed = if text.contains(['e', 'E']) {
        Decimal::from_scientific(text)
    } else {
        Decimal::from_str(text)
    };
    parsed.ok()
}
