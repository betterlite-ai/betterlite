use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;

use crate::auth_key_budgets::money::Money;
use crate::auth_request_models::RequestedModels;

use super::catalog::{PricingCatalog, PricingError, model_info_id};
use super::zero_cost::is_cost_explicitly_configured;

// model_has_no_cost_mapping: a falsey model or a missing router is never unpriced, and an unknown group is not either
pub fn model_has_no_cost_mapping(
    model: Option<&str>,
    catalog: Option<&dyn PricingCatalog>,
) -> Result<bool, PricingError> {
    let (Some(name), Some(pricing)) = (model, catalog) else {
        return Ok(false);
    };
    if name.is_empty() || pricing.model_group_info(name)?.is_none() {
        return Ok(false);
    }
    if model_group_has_pricing(name, pricing)? {
        return Ok(false);
    }
    Ok(!is_cost_explicitly_configured(name, pricing)?)
}

// _unpriced_models_in_request: the model order and its duplicates survive the filter
pub fn unpriced_models_in_request(
    model: &RequestedModels,
    catalog: Option<&dyn PricingCatalog>,
) -> Result<Vec<String>, PricingError> {
    let names: Vec<&str> = match model {
        RequestedModels::None => Vec::new(),
        RequestedModels::One(name) => vec![name.as_str()],
        RequestedModels::Many(names) => names.iter().map(String::as_str).collect(),
    };
    let mut unpriced = Vec::new();
    for name in names {
        if model_has_no_cost_mapping(Some(name), catalog)? {
            unpriced.push(name.to_owned());
        }
    }
    Ok(unpriced)
}

// _unpriced_models_block_message: B754 owns the switch that blocks the request with this text
#[must_use]
pub fn unpriced_models_block_message(models: &[String]) -> String {
    let names = models
        .iter()
        .map(|model| format!("'{model}'"))
        .collect::<Vec<String>>()
        .join(", ");
    let subject = if models.len() == 1 {
        format!("Model {names} has")
    } else {
        format!("Models {names} have")
    };
    format!(
        "{subject} no pricing in the cost map, so litellm cannot price the request. Requests for unpriced models are blocked because 'block_requests_for_models_without_pricing' is enabled. Add pricing (input_cost_per_token/output_cost_per_token) to allow the request."
    )
}

fn model_group_has_pricing(
    model: &str,
    catalog: &dyn PricingCatalog,
) -> Result<bool, PricingError> {
    for deployment in catalog.model_list(model)? {
        if entry_declares_price(&deployment.litellm_params) {
            return Ok(true);
        }
        let Some(model_id) = model_info_id(&deployment.model_info) else {
            continue;
        };
        let model_name = litellm_model_name(&deployment.litellm_params);
        let Some(info) = catalog.deployment_model_info(model_id, model_name)? else {
            continue;
        };
        if entry_has_priced_metric(&info) {
            return Ok(true);
        }
    }
    Ok(false)
}

// litellm_params.get("model") or "": a falsey name reaches get_deployment_model_info as the empty string
fn litellm_model_name(params: &JsonObject) -> &str {
    match params.get("model") {
        Some(Json::Text(name)) => name.as_str(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "",
    }
}

fn entry_declares_price(entry: &JsonObject) -> bool {
    entry
        .keys()
        .any(|key| key.contains("cost_per") || key == "tiered_pricing")
}

// _entry_has_priced_metric: any positive cost_per value counts, nested one level, and tiered_pricing counts when non-null
fn entry_has_priced_metric(entry: &JsonObject) -> bool {
    let tiered = entry
        .get("tiered_pricing")
        .is_some_and(|value| !matches!(value, Json::Null));
    if tiered {
        return true;
    }
    for (key, value) in entry {
        if !key.contains("cost_per") {
            continue;
        }
        if positive_cost(value) {
            return true;
        }
        if let Json::Object(nested) = value
            && nested.values().any(positive_cost)
        {
            return true;
        }
    }
    false
}

// _is_positive_cost: a number above zero, where a string and a bool are not numbers at all
fn positive_cost(value: &Json) -> bool {
    let Json::Number(number) = value else {
        return false;
    };
    match Money::from_text(&number.to_string()) {
        Some(Money::Finite(amount)) => amount > Decimal::ZERO,
        Some(Money::PositiveInfinity) => true,
        Some(Money::NegativeInfinity | Money::Nan) | None => false,
    }
}
