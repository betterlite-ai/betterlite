#![cfg(test)]

use betterlite_models::catalog::BundledCatalog;
use betterlite_models::price_fields::nested::TierPricing;
use rust_decimal::Decimal;

use super::{lookup, pricing_tier};

#[test]
fn lookup_reads_a_flat_query_price() {
    let catalog = BundledCatalog::default().get();
    let info = lookup(&catalog, "dataforseo/search", None).expect("the model resolves");
    assert_eq!(info.input_cost_per_query, Some(Decimal::new(3_i64, 3)));
    assert!(info.tiered_pricing.is_none());
}

#[test]
fn lookup_reads_the_tiered_query_prices() {
    let catalog = BundledCatalog::default().get();
    let info = lookup(&catalog, "exa_ai/search", None).expect("the model resolves");
    let tiers = info.tiered_pricing.expect("the model carries tiers");
    assert_eq!(tiers.len(), 2_usize);
    assert_eq!(tiers[0].max_results_range, (0_i64, 25_i64));
    assert_eq!(tiers[0].input_cost_per_query, Decimal::new(5_i64, 3));
}

#[test]
fn lookup_returns_none_for_a_model_the_catalog_lacks() {
    let catalog = BundledCatalog::default().get();
    assert!(lookup(&catalog, "nonesuch/search", None).is_none());
}

#[test]
fn pricing_tier_requires_both_range_ends_and_a_price() {
    let tier: TierPricing =
        serde_json::from_str(r#"{"max_results_range": [0, 25], "input_cost_per_query": 0.005}"#)
            .expect("the tier parses");
    let parsed = pricing_tier(&tier).expect("the tier names both ends and a price");
    assert_eq!(parsed.max_results_range, (0_i64, 25_i64));
    assert_eq!(parsed.input_cost_per_query, Decimal::new(5_i64, 3));
    let half: TierPricing =
        serde_json::from_str(r#"{"max_results_range": [0], "input_cost_per_query": 0.005}"#)
            .expect("the half tier parses");
    assert!(pricing_tier(&half).is_none());
}
