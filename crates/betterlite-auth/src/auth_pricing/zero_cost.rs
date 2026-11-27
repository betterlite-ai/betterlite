use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use tracing::debug;

use crate::auth_key_budgets::money::Money;
use crate::auth_request_models::RequestedModels;

use super::cache::ZeroCostCache;
use super::catalog::{ModelGroupCosts, PricingCatalog, PricingError, model_info_id};

// _is_model_cost_zero: every candidate must be free; a cached False stops at once and a cached True skips the lookup
#[must_use]
pub fn is_model_cost_zero(model: &RequestedModels, catalog: Option<&dyn PricingCatalog>) -> bool {
    let (Some(pricing), Some(names)) = (catalog, candidates(model)) else {
        return false;
    };
    let cache = pricing.zero_cost_cache();
    for model_name in names {
        if let Some(cached) = cache.and_then(|verdicts| verdicts.get(model_name)) {
            if !cached {
                return false;
            }
            continue;
        }
        if !check_candidate(model_name, pricing, cache) {
            return false;
        }
    }
    true
}

// one uncached model group: logs its verdict, records it, and answers false when the model is not free
fn check_candidate(
    model_name: &str,
    catalog: &dyn PricingCatalog,
    cache: Option<&ZeroCostCache>,
) -> bool {
    let free = match zero_cost_of(model_name, catalog) {
        Ok(free) => free,
        Err(error) => {
            debug!("Error checking cost for model {model_name}: {error}, assuming it has cost");
            return false;
        }
    };
    let Some(costs) = free else {
        remember_not_free(cache, model_name);
        return false;
    };
    debug!(
        "Model {} has zero cost explicitly configured (input: {}, output: {})",
        safe_name(model_name),
        cost_text(costs.input_cost_per_token),
        cost_text(costs.output_cost_per_token)
    );
    if let Some(store) = cache {
        store.record(model_name, true);
    }
    true
}

fn candidates(model: &RequestedModels) -> Option<Vec<&str>> {
    match model {
        RequestedModels::None => None,
        RequestedModels::One(name) => Some(vec![name.as_str()]),
        RequestedModels::Many(names) => Some(names.iter().map(String::as_str).collect()),
    }
}

// the branch LiteLLM settles on for one model group, with the verbose_logger.debug line it prints there
fn zero_cost_of(
    model: &str,
    catalog: &dyn PricingCatalog,
) -> Result<Option<ModelGroupCosts>, PricingError> {
    let Some(costs) = catalog.model_group_info(model)? else {
        debug!("No model group info found for {model}, assuming it has cost");
        return Ok(None);
    };
    let (Some(input), Some(output)) = (costs.input_cost_per_token, costs.output_cost_per_token)
    else {
        debug!(
            "Model {model} has undefined cost (input: {}, output: {}), assuming it has cost",
            cost_text(costs.input_cost_per_token),
            cost_text(costs.output_cost_per_token)
        );
        return Ok(None);
    };
    // LiteLLM compares > 0, so a negative rate is not charged; a non-finite rate is unknown, never free
    if priced_or_unknown(input) || priced_or_unknown(output) {
        debug!(
            "Model {model} has non-zero or non-finite cost (input: {}, output: {})",
            cost_text(costs.input_cost_per_token),
            cost_text(costs.output_cost_per_token)
        );
        return Ok(None);
    }
    if !explicit_zero_cost(model, catalog)? {
        return Ok(None);
    }
    Ok(Some(costs))
}

// the two later gates: the raw entry must declare the zero and no PTU flat cost may apply
fn explicit_zero_cost(model: &str, catalog: &dyn PricingCatalog) -> Result<bool, PricingError> {
    if !is_cost_explicitly_configured(model, catalog)? {
        debug!(
            "Model {} has zero cost but no explicit cost configuration in model_cost entry, treating as unknown cost (enforce budget)",
            safe_name(model)
        );
        return Ok(false);
    }
    // _has_ptu_flat_cost: both fields must be present and non-null, zero values included
    for deployment in catalog.model_list(model)? {
        let ptu_count = deployment
            .model_info
            .get("ptu_count")
            .is_some_and(|value| !matches!(value, Json::Null));
        let hourly = deployment
            .model_info
            .get("cost_per_ptu_per_hour")
            .is_some_and(|value| !matches!(value, Json::Null));
        if ptu_count && hourly {
            debug!(
                "Model {} prices reserved PTU capacity as a flat cost, so its zero per-token rate is not a free model (enforce budget)",
                safe_name(model)
            );
            return Ok(false);
        }
    }
    Ok(true)
}

fn priced_or_unknown(cost: Money) -> bool {
    match cost {
        Money::Finite(amount) => amount > Decimal::ZERO,
        Money::Nan | Money::PositiveInfinity | Money::NegativeInfinity => true,
    }
}

// _is_cost_explicitly_configured: either raw token-price key counts even when its value is null
pub(super) fn is_cost_explicitly_configured(
    model: &str,
    catalog: &dyn PricingCatalog,
) -> Result<bool, PricingError> {
    for deployment in catalog.model_list(model)? {
        let Some(model_id) = model_info_id(&deployment.model_info) else {
            continue;
        };
        let Some(entry) = catalog.raw_cost_entry(model_id) else {
            continue;
        };
        if entry.contains_key("input_cost_per_token") || entry.contains_key("output_cost_per_token")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn remember_not_free(store: Option<&ZeroCostCache>, model: &str) {
    if let Some(cache) = store {
        cache.record(model, false);
    }
}

// the explicit-cost, PTU and success lines carry the model name, so a newline in it cannot forge a log line
fn safe_name(model: &str) -> String {
    model.replace(['\n', '\r'], "")
}

fn cost_text(cost: Option<Money>) -> String {
    match cost {
        Some(Money::Finite(amount)) => amount.to_string(),
        Some(Money::Nan) => "NaN".to_owned(),
        Some(Money::PositiveInfinity) => "Infinity".to_owned(),
        Some(Money::NegativeInfinity) => "-Infinity".to_owned(),
        None => "None".to_owned(),
    }
}
