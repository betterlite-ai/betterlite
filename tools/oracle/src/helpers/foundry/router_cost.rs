use std::collections::BTreeMap;

use betterlite_foundry::pricing::error::PricingError;
use betterlite_foundry::pricing::fee::router_flat_cost;
use betterlite_foundry::pricing::names::{classify_router_name, router_fee_name};
use betterlite_foundry::pricing::response::{ResponseCostRequest, cost_per_token};
use betterlite_models::cost_usage::CostUsage;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde_json::{Value, json};

use crate::helpers::HelperError;

mod input;

use input::{ClassifyCall, CostCall, FlatCall, Input};

// Mirrors get_model_info: the provider-prefixed key first, then the bare model name.
struct CatalogLookup {
    entries: BTreeMap<String, PriceRecord>,
}

impl CatalogLookup {
    fn resolve(&self, model: &str, provider: Option<&str>) -> Option<PriceRecord> {
        provider
            .map(|provider_name| format!("{provider_name}/{model}"))
            .and_then(|prefixed| self.entries.get(&prefixed).cloned())
            .or_else(|| self.entries.get(model).cloned())
    }
}

impl PriceLookup for CatalogLookup {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.resolve(model, custom_llm_provider)
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.resolve(model, custom_llm_provider)
            .map(|record| PricedModel {
                record,
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            })
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        match input {
            Input::Classify { calls } => Ok(classify(calls)),
            Input::Flat { catalog, calls } => {
                let lookup = CatalogLookup {
                    entries: catalog.clone(),
                };
                flat(&lookup, calls)
            }
            Input::Cost { catalog, calls } => {
                let lookup = CatalogLookup {
                    entries: catalog.clone(),
                };
                cost(&lookup, calls)
            }
        }
    }
}

fn classify(calls: &[ClassifyCall]) -> Value {
    let reports: Vec<Value> = calls
        .iter()
        .map(|call| {
            let name = classify_router_name(&call.model);
            json!({
                "is_router": name.is_router_shaped(),
                "is_fee_entry": name.is_fee_entry(),
                "fee_entry_name": name.fee_lookup_name(),
                "fee_name": router_fee_name(&call.model, call.request_model.as_deref()),
            })
        })
        .collect();
    Value::Array(reports)
}

fn flat(lookup: &CatalogLookup, calls: &[FlatCall]) -> crate::helpers::HelperResult {
    let mut reports = Vec::with_capacity(calls.len());
    for call in calls {
        let fee = router_flat_cost(&call.model, call.prompt_tokens, lookup)
            .map_err(|error| pricing_error(&error))?;
        reports.push(json!({ "fee": fee.normalize().to_string() }));
    }
    Ok(Value::Array(reports))
}

fn cost(lookup: &CatalogLookup, calls: &[CostCall]) -> crate::helpers::HelperResult {
    let database = TimeZoneDatabase::from_env();
    let prompt_extras = PromptExtras::default();
    let mut reports = Vec::with_capacity(calls.len());
    for call in calls {
        let usage = CostUsage {
            prompt_tokens: call.prompt_tokens,
            completion_tokens: call.completion_tokens,
            total_tokens: 0,
            prompt_details: None,
            completion_details: None,
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        };
        let request = ResponseCostRequest {
            model: &call.model,
            usage: &usage,
            prompt_extras: &prompt_extras,
            response_time_ms: call.response_time_ms,
            request_model: call.request_model.as_deref(),
            service_tier: call.service_tier.as_deref(),
            // off-peak windows never reach an azure_ai router entry, so the answer is time-independent
            moment: DateTime::<Utc>::UNIX_EPOCH,
        };
        let result =
            cost_per_token(&request, lookup, &database).map_err(|error| pricing_error(&error))?;
        reports.push(json!({
            "prompt_cost": result.prompt_cost.normalize().to_string(),
            "completion_cost": result.completion_cost.normalize().to_string(),
        }));
    }
    Ok(Value::Array(reports))
}

fn pricing_error(error: &PricingError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "PricingError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
