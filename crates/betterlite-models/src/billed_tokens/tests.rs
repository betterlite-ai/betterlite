#![cfg(test)]

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

use super::{
    BilledTokenCounts, BilledTokenRequest, BilledTokenUsage, breakdown, caching_savings,
    catalog_billed_token_rates, custom_billed_token_rates, custom_cost,
    project_billed_token_counts,
};
use crate::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use crate::tiered_pricing::Scalar;

struct Lookup(Option<PricedModel>);

impl PriceLookup for Lookup {
    fn effective_model_info(&self, _model: &str, _provider: Option<&str>) -> Option<PriceRecord> {
        self.0.as_ref().map(|model| model.record.clone())
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(&self, _model: &str, _provider: Option<&str>) -> Option<PricedModel> {
        self.0.clone()
    }
}

fn decimal(value: &str) -> Decimal {
    Decimal::from_str_exact(value).expect("test decimal is valid")
}

fn billing_request(pricing: Option<&PricedModel>) -> BilledTokenRequest<'_> {
    BilledTokenRequest {
        model: "oracle-model",
        provider: None,
        pricing,
        service_tier: None,
        residency: None,
        vertex_location: None,
        inference_geo: None,
        provider_specific_entry: None,
        moment: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is in range"),
        prompt_tokens: 10,
    }
}

#[test]
fn catalog_lookup_miss_returns_no_rates() {
    let database = TimeZoneDatabase::from_env();
    let lookup = Lookup(None);
    let request = billing_request(None);
    assert_eq!(
        catalog_billed_token_rates(&request, &lookup, &database)
            .expect("lookup miss is a no-rate result"),
        None
    );
}

#[test]
fn malformed_catalog_rate_propagates_after_successful_lookup() {
    let database = TimeZoneDatabase::from_env();
    let pricing = PricedModel {
        record: BTreeMap::from([
            (
                "input_cost_per_token".to_owned(),
                Scalar::Text("1e999".to_owned()),
            ),
            (
                "output_cost_per_token".to_owned(),
                Scalar::Text("0.2".to_owned()),
            ),
        ]),
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    };
    let request = billing_request(Some(&pricing));
    let error = catalog_billed_token_rates(&request, &Lookup(None), &database)
        .expect_err("malformed resolved pricing remains an error");
    assert!(error.to_string().contains("invalid exact rate"));
}

#[test]
fn zero_detail_counters_fall_back_to_positive_top_level_counts() {
    let counts = project_billed_token_counts(BilledTokenUsage {
        detail_reasoning: Some(0),
        top_level_reasoning: Some(4),
        detail_cache_read: Some(0),
        top_level_cache_read: Some(5),
        detail_cache_creation: Some(0),
        top_level_cache_creation: Some(3),
        detail_cached_audio: Some(2),
        detail_cache_creation_5m: Some(1),
        detail_cache_creation_1h: Some(2),
    });
    assert_eq!(counts.reasoning, 4);
    assert_eq!(counts.cache_read, 5);
    assert_eq!(counts.cache_creation, 3);
    assert_eq!(counts.cached_audio, 2);
    assert_eq!(counts.cache_creation_5m, Some(1));
    assert_eq!(counts.cache_creation_1h, Some(2));
}

#[test]
fn cache_savings_preserves_free_reads_and_negative_write_premiums() {
    let savings = caching_savings(
        decimal("0.1"),
        Some(Decimal::ZERO),
        Some(Decimal::ZERO),
        Some(Decimal::ZERO),
        4,
        2,
        Some(1),
        Some(1),
        Decimal::ONE,
    )
    .expect("valid arithmetic");
    assert_eq!(savings, decimal("0.4"));

    let premium = caching_savings(
        decimal("0.1"),
        None,
        Some(decimal("0.2")),
        Some(decimal("0.3")),
        0,
        2,
        Some(1),
        Some(1),
        Decimal::ONE,
    )
    .expect("valid arithmetic");
    assert_eq!(premium, decimal("-0.3"));
}

#[test]
fn custom_flat_rates_preserve_free_cache_and_use_output_for_reasoning() {
    let rates =
        custom_billed_token_rates(decimal("0.1"), decimal("0.2"), Some(Decimal::ZERO), None);
    assert_eq!(rates.audio_read, Decimal::ZERO);
    assert_eq!(rates.cache_write_1h, decimal("0.1"));
    assert_eq!(rates.reasoning, decimal("0.2"));
}

#[test]
fn custom_token_cost_wins_over_seconds_and_counts_cached_tokens_in_prompt() {
    let cost = custom_cost(
        10,
        5,
        2,
        3,
        Some((decimal("0.1"), decimal("0.2"), Some(decimal("0.05")), None)),
        Some(decimal("100")),
        Some(decimal("1000")),
    )
    .expect("valid arithmetic")
    .expect("custom pricing supplied");
    assert_eq!(cost, (decimal("0.9"), decimal("1")));
}

#[test]
fn breakdown_returns_the_rates_used_for_each_cost_line() {
    let rates = custom_billed_token_rates(decimal("0.1"), decimal("0.2"), None, None);
    let result = breakdown(
        BilledTokenCounts {
            reasoning: 2,
            cache_read: 4,
            cached_audio: 1,
            cache_creation: 3,
            cache_creation_5m: Some(2),
            cache_creation_1h: Some(1),
        },
        Some(rates),
    )
    .expect("valid arithmetic");
    assert_eq!(result.reasoning_cost, decimal("0.4"));
    assert_eq!(result.cache_read_cost, decimal("0.4"));
    assert_eq!(result.cache_creation_cost, decimal("0.3"));
    assert_eq!(result.rates, Some(rates));
}
