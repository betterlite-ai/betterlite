use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_cost::search::SearchPricingSource;
use betterlite_cost::search::model_info::{PricingTier, SearchModelInfo};
use betterlite_models::catalog::{CatalogEntry, ModelCatalog};
use betterlite_models::model_catalog_keys::{
    EntryProvider, check_provider_match, strip_model_name,
};
use betterlite_models::price_fields::nested::TierPricing;
use rust_decimal::prelude::ToPrimitive as _;

// get_model_info reports an unknown model softly; search_provider_cost_per_query raises it as ModelNotMappedError.
#[derive(Debug, thiserror::Error)]
#[error("{0} isn't mapped yet")]
pub(crate) struct Unmapped(pub(crate) String);

pub(crate) struct CatalogPricing {
    pub(crate) catalog: Arc<ModelCatalog>,
}

// The keys _get_model_info_helper tries, in its order, for the model and provider the caller named.
struct CandidateNames {
    combined_model_name: String,
    split_model: String,
    combined_stripped: String,
    stripped: String,
    provider_prefixed: String,
}

fn candidate_names(model: &str, provider: Option<&str>) -> CandidateNames {
    let Some(provider) = provider else {
        let stripped = strip_model_name(model, None, str::to_owned);
        return CandidateNames {
            combined_model_name: model.to_owned(),
            split_model: model.to_owned(),
            combined_stripped: stripped.clone(),
            stripped,
            provider_prefixed: model.to_owned(),
        };
    };
    let prefix = format!("{provider}/");
    let prefixed = model.strip_prefix(&prefix);
    let split_model = prefixed.unwrap_or(model);
    let stripped = strip_model_name(split_model, Some(provider), str::to_owned);
    CandidateNames {
        combined_model_name: prefixed.map_or_else(|| format!("{provider}/{model}"), str::to_owned),
        split_model: split_model.to_owned(),
        combined_stripped: format!("{provider}/{stripped}"),
        stripped,
        provider_prefixed: format!("{provider}/{model}"),
    }
}

impl SearchPricingSource for CatalogPricing {
    type Error = Unmapped;

    fn model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> impl Future<Output = Result<SearchModelInfo, Unmapped>> + Send {
        ready(
            self.lookup(model, custom_llm_provider)
                .ok_or_else(|| Unmapped(model.to_owned())),
        )
    }
}

impl CatalogPricing {
    fn lookup(&self, model: &str, provider: Option<&str>) -> Option<SearchModelInfo> {
        let names = candidate_names(model, provider);
        let candidates = [
            names.combined_model_name.as_str(),
            model,
            names.split_model.as_str(),
            names.combined_stripped.as_str(),
            names.stripped.as_str(),
            names.provider_prefixed.as_str(),
        ];
        for candidate in candidates {
            let Some(entry) = self.entry(candidate) else {
                continue;
            };
            let entry_provider = entry.fields.provider_projection().litellm_provider;
            let matched = match check_provider_match(EntryProvider::Text(entry_provider), provider)
            {
                Ok(matched) => matched,
                Err(_error) => false,
            };
            if matched {
                return Some(model_info(entry));
            }
        }
        None
    }

    // LiteLLM's lookup is exact first and case-insensitive after; the catalog keeps both indexes.
    fn entry(&self, name: &str) -> Option<&CatalogEntry> {
        if let Some(entry) = self.catalog.entries.get(name) {
            return Some(entry);
        }
        let canonical = self.catalog.lookup.get(&name.to_lowercase())?;
        self.catalog.entries.get(canonical.as_ref())
    }
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
