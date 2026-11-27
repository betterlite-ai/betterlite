use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use tracing::debug;

use crate::auth_key_budgets::money::Money;
use crate::auth_request_models::RequestedModels;

use super::cache::ZeroCostCache;
use super::catalog::{ModelGroupCosts, PricingCatalog, PricingError, model_info_id};

// the branch _is_model_cost_zero settles on for one model group before it logs and caches
enum Verdict {
    Free(ModelGroupCosts),
    MissingGroup,
    Undefined(ModelGroupCosts),
    Charged(ModelGroupCosts),
    NotExplicit,
    Ptu,
}

// _is_model_cost_zero: every candidate must be free; a cached False stops at once and a cached True skips the lookup
#[must_use]
pub fn is_model_cost_zero(model: &RequestedModels, catalog: Option<&dyn PricingCatalog>) -> bool {
    let (Some(catalog), Some(names)) = (catalog, candidates(model)) else {
        return false;
    };
    let cache = catalog.zero_cost_cache();
    for model_name in names {
        if let Some(cached) = cache.and_then(|cache| cache.get(model_name)) {
            if !cached {
                return false;
            }
            continue;
        }
        let verdict = match zero_cost_of(model_name, catalog) {
            Ok(verdict) => verdict,
            Err(error) => {
                debug!("Error checking cost for model {model_name}: {error}, assuming it has cost");
                return false;
            }
        };
        match verdict {
            Verdict::Free(costs) => {
                let safe = safe_name(model_name);
                debug!(
                    "Model {safe} has zero cost explicitly configured (input: {}, output: {})",
                    cost_text(costs.input_cost_per_token),
                    cost_text(costs.output_cost_per_token)
                );
                if let Some(cache) = cache {
                    cache.record(model_name, true);
                }
            }
            Verdict::MissingGroup => {
                debug!("No model group info found for {model_name}, assuming it has cost");
                remember_not_free(cache, model_name);
                return false;
            }
            Verdict::Undefined(costs) => {
                debug!(
                    "Model {model_name} has undefined cost (input: {}, output: {}), assuming it has cost",
                    cost_text(costs.input_cost_per_token),
                    cost_text(costs.output_cost_per_token)
                );
                remember_not_free(cache, model_name);
                return false;
            }
            Verdict::Charged(costs) => {
                debug!(
                    "Model {model_name} has non-zero or non-finite cost (input: {}, output: {})",
                    cost_text(costs.input_cost_per_token),
                    cost_text(costs.output_cost_per_token)
                );
                remember_not_free(cache, model_name);
                return false;
            }
            Verdict::NotExplicit => {
                let safe = safe_name(model_name);
                debug!(
                    "Model {safe} has zero cost but no explicit cost configuration in model_cost entry, treating as unknown cost (enforce budget)"
                );
                remember_not_free(cache, model_name);
                return false;
            }
            Verdict::Ptu => {
                let safe = safe_name(model_name);
                debug!(
                    "Model {safe} prices reserved PTU capacity as a flat cost, so its zero per-token rate is not a free model (enforce budget)"
                );
                remember_not_free(cache, model_name);
                return false;
            }
        }
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

fn zero_cost_of(model: &str, catalog: &dyn PricingCatalog) -> Result<Verdict, PricingError> {
    let Some(costs) = catalog.model_group_info(model)? else {
        return Ok(Verdict::MissingGroup);
    };
    let (Some(input), Some(output)) = (costs.input_cost_per_token, costs.output_cost_per_token)
    else {
        return Ok(Verdict::Undefined(costs));
    };
    // LiteLLM compares > 0, so a negative rate is not charged; a non-finite rate is unknown, never free
    if priced_or_unknown(input) || priced_or_unknown(output) {
        return Ok(Verdict::Charged(costs));
    }
    if !is_cost_explicitly_configured(model, catalog)? {
        return Ok(Verdict::NotExplicit);
    }
    if has_ptu_flat_cost(model, catalog)? {
        return Ok(Verdict::Ptu);
    }
    Ok(Verdict::Free(costs))
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

// _has_ptu_flat_cost: both fields must be present and non-null, zero values included
fn has_ptu_flat_cost(model: &str, catalog: &dyn PricingCatalog) -> Result<bool, PricingError> {
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
            return Ok(true);
        }
    }
    Ok(false)
}

fn remember_not_free(cache: Option<&ZeroCostCache>, model: &str) {
    if let Some(cache) = cache {
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
