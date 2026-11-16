use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_provider::databricks_cost::{CostRequest, PROVIDER, cost_per_token};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use serde_json::json;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::{CostCase, RateCase};

struct CasePrices<'case> {
    rates: &'case BTreeMap<String, RateCase>,
}

impl PriceLookup for CasePrices<'_> {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        // only the mapped key under the databricks provider prices, so the lookup proves both
        if custom_llm_provider != Some(PROVIDER) {
            return None;
        }
        let rates = self.rates.get(model)?;
        Some(PricedModel {
            record: BTreeMap::from([
                (
                    "input_cost_per_token".to_owned(),
                    Scalar::Text(rates.input_cost_per_token.clone()),
                ),
                (
                    "output_cost_per_token".to_owned(),
                    Scalar::Text(rates.output_cost_per_token.clone()),
                ),
            ]),
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

pub(super) fn cost(cases: &[CostCase]) -> HelperResult {
    let database = TimeZoneDatabase::from_env();
    let usage = CostUsage {
        prompt_tokens: 1_000,
        completion_tokens: 1_000,
        total_tokens: 2_000,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    };
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let lookup = CasePrices {
            rates: &case.pricing,
        };
        let mut priced = Vec::with_capacity(case.models.len());
        for model in &case.models {
            let request = CostRequest {
                model,
                usage: &usage,
                service_tier: case.service_tier.as_deref(),
                moment: DateTime::<Utc>::UNIX_EPOCH,
            };
            let cost = cost_per_token(&request, &lookup, &database).or_value_error()?;
            priced.push(json!({
                "prompt": cost.prompt_cost.normalize().to_string(),
                "completion": cost.completion_cost.normalize().to_string(),
            }));
        }
        results.push(json!(priced));
    }
    Ok(json!(results))
}
