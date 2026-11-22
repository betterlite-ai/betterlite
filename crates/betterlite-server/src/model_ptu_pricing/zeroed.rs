use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_fields::nested::SearchContextPricing;
use betterlite_models::ptu_pricing::pricing_fields::{
    CUSTOM_PRICING_FIELDS, SEARCH_CONTEXT_QUERY_FIELD, TIERED_PRICING_FIELD, ZEROED_RATE_FIELDS,
};
use betterlite_models::ptu_pricing::zeroed::{PtuZero, PtuZeroedPricing};
use rust_decimal::Decimal;
use serde_json::Number;

use super::error::PtuPricingError;
use super::prices::is_nonzero_price;
use super::{DeploymentBlobs, PTU_PRICED_PAIR};
use crate::jobs::ptu::attribution::PtuAttribution;

// scale 1 keeps the zero's text 0.0, the float LiteLLM writes into both blobs
const ZERO_RATE: Decimal = Decimal::from_parts(0, 0, 0, false, 1);

fn bills_reserved_capacity(model_info: &JsonObject, attribution: &dyn PtuAttribution) -> bool {
    attribution.enabled()
        && PTU_PRICED_PAIR.into_iter().all(|field| {
            model_info
                .get(field)
                .is_some_and(|value| !matches!(value, Json::Null))
        })
}

// only a price the caller supplies is refused; one already stored is zeroed on the next save instead
pub fn check_ptu_unpriced(
    model_info: &JsonObject,
    supplied: &JsonObject,
    attribution: &dyn PtuAttribution,
) -> Result<(), PtuPricingError> {
    if !bills_reserved_capacity(model_info, attribution) {
        return Ok(());
    }
    let mut priced: BTreeSet<&'static str> = CUSTOM_PRICING_FIELDS
        .into_iter()
        .filter(|field| supplied.get(*field).is_some_and(is_nonzero_price))
        .collect();
    let tiers_supplied = match supplied.get(TIERED_PRICING_FIELD) {
        None | Some(Json::Null) => false,
        Some(Json::List(tiers)) => !tiers.is_empty(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => true,
    };
    if tiers_supplied {
        priced.insert(TIERED_PRICING_FIELD);
    }
    if priced.is_empty() {
        return Ok(());
    }
    Err(PtuPricingError::Priced {
        priced: priced.into_iter().collect(),
    })
}

// empty unless the deployment bills reserved capacity, since per-token rates would charge the same traffic twice
pub fn ptu_zeroed_pricing(
    blobs: DeploymentBlobs<'_>,
    supplied: &JsonObject,
    attribution: &dyn PtuAttribution,
) -> Result<PtuZeroedPricing, PtuPricingError> {
    let mut entries = BTreeMap::new();
    if !bills_reserved_capacity(blobs.model_info, attribution) {
        return Ok(PtuZeroedPricing { entries });
    }
    check_ptu_unpriced(blobs.model_info, supplied, attribution)?;
    for field in ZEROED_RATE_FIELDS {
        entries.insert(field, PtuZero::Rate(ZERO_RATE));
    }
    entries.insert(TIERED_PRICING_FIELD, PtuZero::EmptyTiers);
    // an absent search table falls back to the provider's own rate, so it is zeroed in place on every PTU deployment
    let free_search = SearchContextPricing {
        search_context_size_low: Some(ZERO_RATE),
        search_context_size_medium: Some(ZERO_RATE),
        search_context_size_high: Some(ZERO_RATE),
    };
    entries.insert(
        SEARCH_CONTEXT_QUERY_FIELD,
        PtuZero::SearchContext(free_search),
    );
    for field in CUSTOM_PRICING_FIELDS {
        let stored = [blobs.model_info, blobs.litellm_params]
            .into_iter()
            .any(|blob| blob.get(field).is_some_and(is_nonzero_price));
        if stored && field != SEARCH_CONTEXT_QUERY_FIELD {
            entries.insert(field, PtuZero::Rate(ZERO_RATE));
        }
    }
    Ok(PtuZeroedPricing { entries })
}

pub fn write_ptu_pricing(
    pricing: &PtuZeroedPricing,
    blob: &mut JsonObject,
) -> Result<(), PtuPricingError> {
    for (field, zero) in &pricing.entries {
        let value = match zero {
            PtuZero::Rate(rate) => rate_json(*rate)?,
            PtuZero::EmptyTiers => Json::List(Vec::new()),
            PtuZero::SearchContext(table) => search_context_json(table)?,
        };
        blob.insert((*field).to_owned(), value);
    }
    Ok(())
}

// the deployment add_new_model stores: its own litellm_params are both the supplied and the stored prices
pub fn ptu_priced_deployment(
    litellm_params: &mut JsonObject,
    model_info: &mut JsonObject,
    attribution: &dyn PtuAttribution,
) -> Result<(), PtuPricingError> {
    let blobs = DeploymentBlobs {
        model_info,
        litellm_params,
    };
    let pricing = ptu_zeroed_pricing(blobs, blobs.litellm_params, attribution)?;
    write_ptu_pricing(&pricing, litellm_params)?;
    write_ptu_pricing(&pricing, model_info)
}

fn search_context_json(table: &SearchContextPricing) -> Result<Json, PtuPricingError> {
    let sizes = [
        ("search_context_size_low", table.search_context_size_low),
        (
            "search_context_size_medium",
            table.search_context_size_medium,
        ),
        ("search_context_size_high", table.search_context_size_high),
    ];
    let mut rates = JsonObject::new();
    for (size, rate) in sizes {
        if let Some(price) = rate {
            rates.insert(size.to_owned(), rate_json(price)?);
        }
    }
    Ok(Json::Object(rates))
}

fn rate_json(rate: Decimal) -> Result<Json, PtuPricingError> {
    Number::from_str(&rate.to_string())
        .map(Json::Number)
        .map_err(|source| PtuPricingError::Rate { rate, source })
}
