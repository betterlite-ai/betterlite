use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_cost::search::SearchPricingSource;
use betterlite_cost::search::model_info::{PricingTier, SearchModelInfo};
use betterlite_models::catalog::{CatalogEntry, ModelCatalog};
use betterlite_models::lookup::info::ModelQuery;
use betterlite_models::lookup::resolve::find_info;
use betterlite_models::price_fields::nested::TierPricing;
use betterlite_tokenizer::model_capabilities::BEDROCK_NAMES;
use rust_decimal::prelude::ToPrimitive as _;

// get_model_info raises for a model the catalog cannot resolve; search_provider_cost_per_query keeps its text.
#[derive(Debug, thiserror::Error)]
#[error("{0} isn't mapped yet")]
pub(crate) struct Unmapped(pub(crate) String);

pub(crate) struct CatalogPricing {
    pub(crate) catalog: Arc<ModelCatalog>,
}

impl SearchPricingSource for CatalogPricing {
    type Error = Unmapped;

    fn model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> impl Future<Output = Result<SearchModelInfo, Unmapped>> + Send {
        ready(
            lookup(&self.catalog, model, custom_llm_provider)
                .ok_or_else(|| Unmapped(model.to_owned())),
        )
    }
}

// get_model_info: the entry the model resolves to; a model only a routing rule knows carries no price of its own.
fn lookup(catalog: &ModelCatalog, model: &str, provider: Option<&str>) -> Option<SearchModelInfo> {
    let query = ModelQuery { model, provider };
    let resolved = find_info(catalog, &BEDROCK_NAMES, &query).ok()??;
    let Some(entry) = catalog.entries.get(resolved.key.as_str()) else {
        return Some(SearchModelInfo::default());
    };
    Some(model_info(entry))
}

fn model_info(entry: &CatalogEntry) -> SearchModelInfo {
    let tiers: Vec<PricingTier> = entry.prices.tiers.iter().filter_map(pricing_tier).collect();
    SearchModelInfo {
        input_cost_per_query: entry.prices.values.input.input_query,
        tiered_pricing: (!tiers.is_empty()).then_some(tiers),
    }
}

// LiteLLM reads a tier only when it names both ends and its own per-query price.
fn pricing_tier(tier: &TierPricing) -> Option<PricingTier> {
    let [minimum, maximum] = tier.max_results_range.as_deref()? else {
        return None;
    };
    Some(PricingTier {
        max_results_range: (minimum.to_i64()?, maximum.to_i64()?),
        input_cost_per_query: tier.input_cost_per_query?,
    })
}
