use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;

use crate::price_fields::nested::SearchContextPricing;
use crate::ptu_pricing::pricing_fields::{
    CUSTOM_PRICING_FIELDS, SEARCH_CONTEXT_QUERY_FIELD, TIERED_PRICING_FIELD, ZEROED_RATE_FIELDS,
};
use crate::ptu_pricing::{PtuModelInfo, ptu_terms};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PtuZero {
    Rate(Decimal),
    EmptyTiers,
    SearchContext(SearchContextPricing),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PtuZeroedPricing {
    pub entries: BTreeMap<&'static str, PtuZero>,
}

#[must_use]
pub fn zeroed_ptu_pricing(
    model_info: &PtuModelInfo<'_>,
    declared: &BTreeSet<String>,
    enabled: bool,
) -> Option<PtuZeroedPricing> {
    ptu_terms(model_info)?;
    if !enabled {
        return None;
    }
    let mut entries: BTreeMap<&'static str, PtuZero> = ZEROED_RATE_FIELDS
        .iter()
        .map(|field| (*field, PtuZero::Rate(Decimal::ZERO)))
        .collect();
    entries.insert(TIERED_PRICING_FIELD, PtuZero::EmptyTiers);
    entries.insert(
        SEARCH_CONTEXT_QUERY_FIELD,
        PtuZero::SearchContext(SearchContextPricing {
            search_context_size_low: Some(Decimal::ZERO),
            search_context_size_medium: Some(Decimal::ZERO),
            search_context_size_high: Some(Decimal::ZERO),
        }),
    );
    for field in CUSTOM_PRICING_FIELDS {
        if field != SEARCH_CONTEXT_QUERY_FIELD && declared.contains(field) {
            entries.insert(field, PtuZero::Rate(Decimal::ZERO));
        }
    }
    Some(PtuZeroedPricing { entries })
}
