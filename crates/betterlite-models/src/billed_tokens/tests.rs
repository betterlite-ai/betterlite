#![cfg(test)]

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

use super::catalog::catalog_billed_token_rates;
use super::custom::{CustomTokenUsage, custom_billed_token_rates, custom_breakdown, custom_cost};
use super::savings::{caching_savings, model_caching_savings};
use super::{
    BilledTokenCounts, BilledTokenRequest, BilledTokenUsage, breakdown, project_billed_token_counts,
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
    assert_eq!(counts.cached_audio, 0);
    assert_eq!(counts.cache_creation_5m, Some(1));
    assert_eq!(counts.cache_creation_1h, Some(2));
}

#[test]
fn cache_savings_preserves_free_reads_and_negative_write_premiums() {
    let counts = || BilledTokenCounts {
        reasoning: 0,
        cache_read: 4,
        cached_audio: 0,
        cache_creation: 2,
        cache_creation_5m: Some(1),
        cache_creation_1h: Some(1),
    };
    let rates = custom_billed_token_rates(
        decimal("0.1"),
        Decimal::ZERO,
        Some(Decimal::ZERO),
        Some(Decimal::ZERO),
    );
    let savings = caching_savings(rates, counts(), Decimal::ONE).unwrap();
    assert_eq!(savings, decimal("0.4"));
    let mut premium_rates =
        custom_billed_token_rates(decimal("0.1"), Decimal::ZERO, None, Some(decimal("0.2")));
    premium_rates.cache_write_1h = decimal("0.3");
    let premium = caching_savings(premium_rates, counts(), Decimal::ONE).unwrap();
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
        CustomTokenUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            cached_tokens: 2,
            cache_creation_tokens: 3,
        },
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

#[rstest::rstest]
#[case::missing(None, None, None)]
#[case::explicit_free_seconds(Some("0"), Some("1500"), Some("0"))]
#[case::missing_duration(Some("2"), None, Some("0"))]
#[case::negative_duration(Some("2"), Some("-1500"), Some("-3"))]
fn seconds_only_or_absent_custom_pricing(
    #[case] price: Option<&str>,
    #[case] duration: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        custom_cost(
            CustomTokenUsage::default(),
            None,
            price.map(decimal),
            duration.map(decimal)
        )
        .unwrap(),
        expected.map(|value| (Decimal::ZERO, decimal(value)))
    );
}

#[test]
fn custom_breakdown_ignores_ttl_buckets() {
    let counts = project_billed_token_counts(BilledTokenUsage {
        detail_cache_creation: Some(9),
        detail_cache_creation_5m: Some(1),
        detail_cache_creation_1h: Some(2),
        detail_reasoning: None,
        top_level_reasoning: None,
        detail_cache_read: None,
        top_level_cache_read: None,
        top_level_cache_creation: None,
        detail_cached_audio: None,
    });
    let rates = custom_billed_token_rates(Decimal::ONE, decimal("2"), None, Some(decimal("3")));
    assert_eq!(
        custom_breakdown(counts, rates).unwrap().cache_creation_cost,
        decimal("27")
    );
}

#[test]
fn catalog_partial_ttl_record_replaces_aggregate() {
    let counts = project_billed_token_counts(BilledTokenUsage {
        detail_cache_creation: Some(9),
        detail_cache_creation_5m: None,
        detail_cache_creation_1h: Some(2),
        detail_reasoning: None,
        top_level_reasoning: None,
        detail_cache_read: None,
        top_level_cache_read: None,
        top_level_cache_creation: None,
        detail_cached_audio: None,
    });
    let rates = custom_billed_token_rates(Decimal::ONE, decimal("2"), None, Some(decimal("3")));
    assert_eq!(
        breakdown(counts, Some(rates)).unwrap().cache_creation_cost,
        decimal("6")
    );
}

#[test]
fn no_rates_returns_three_zeros() {
    let counts = project_billed_token_counts(BilledTokenUsage::default());
    let result = breakdown(counts, None).unwrap();
    assert_eq!(result.rates, None);
    assert_eq!(
        (
            result.reasoning_cost,
            result.cache_read_cost,
            result.cache_creation_cost
        ),
        (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO)
    );
}

#[test]
fn scaling_one_preserves_rates_and_overflow_is_an_error() {
    let rates = custom_billed_token_rates(Decimal::MAX, Decimal::ONE, None, None);
    assert_eq!(rates.scaled(Decimal::ONE).unwrap(), rates);
    assert!(matches!(
        rates.scaled(decimal("2")).unwrap_err(),
        super::BilledTokenError::Overflow
    ));
}

#[test]
fn malformed_geo_multiplier_propagates_after_lookup() {
    let database = TimeZoneDatabase::from_env();
    let pricing = PricedModel::default();
    let geo = BTreeMap::from([("us".to_owned(), Scalar::Text("invalid".to_owned()))]);
    let mut request = billing_request(Some(&pricing));
    request.inference_geo = Some("us");
    request.provider_specific_entry = Some(&geo);
    let error = catalog_billed_token_rates(&request, &Lookup(None), &database).unwrap_err();
    assert!(matches!(error, super::BilledTokenError::TokenCost(_)));
}

#[test]
fn custom_cache_counts_clamp_regular_prompt_to_zero() {
    let usage = CustomTokenUsage {
        prompt_tokens: 1,
        completion_tokens: 2,
        cached_tokens: 3,
        cache_creation_tokens: 4,
    };
    let result = custom_cost(
        usage,
        Some((Decimal::ONE, decimal("2"), None, Some(Decimal::ZERO))),
        None,
        None,
    )
    .unwrap();
    assert_eq!(result, Some((decimal("3"), decimal("4"))));
}

#[test]
fn savings_clamps_negative_counts_and_read_surcharge() {
    let counts = BilledTokenCounts {
        reasoning: 0,
        cache_read: 4,
        cached_audio: 0,
        cache_creation: -5,
        cache_creation_5m: Some(-2),
        cache_creation_1h: Some(-3),
    };
    let rates = custom_billed_token_rates(
        Decimal::ONE,
        Decimal::ONE,
        Some(decimal("2")),
        Some(decimal("3")),
    );
    assert_eq!(
        caching_savings(rates, counts, Decimal::ONE).unwrap(),
        Decimal::ZERO
    );
}

#[test]
fn catalog_lookup_uses_supplied_provider_geo_metadata() {
    let database = TimeZoneDatabase::from_env();
    let mut priced = PricedModel::default();
    priced.record.insert(
        "input_cost_per_token".to_owned(),
        Scalar::Text("2".to_owned()),
    );
    let geo = BTreeMap::from([("us".to_owned(), Scalar::Text("3".to_owned()))]);
    let mut request = billing_request(None);
    request.inference_geo = Some("us");
    request.provider_specific_entry = Some(&geo);
    let rates = catalog_billed_token_rates(&request, &Lookup(Some(priced)), &database)
        .unwrap()
        .unwrap();
    assert_eq!(rates.input_cost_per_token, decimal("6"));
}

#[test]
fn savings_uses_detail_counts_without_private_fallback() {
    let database = TimeZoneDatabase::from_env();
    let mut priced = PricedModel::default();
    priced.record.insert(
        "input_cost_per_token".to_owned(),
        Scalar::Text("2".to_owned()),
    );
    priced.record.insert(
        "cache_read_input_token_cost".to_owned(),
        Scalar::Text("1".to_owned()),
    );
    let request = billing_request(Some(&priced));
    let usage = BilledTokenUsage {
        detail_reasoning: None,
        top_level_reasoning: None,
        detail_cache_read: Some(0),
        top_level_cache_read: Some(10),
        detail_cache_creation: Some(0),
        top_level_cache_creation: Some(20),
        detail_cached_audio: None,
        detail_cache_creation_5m: None,
        detail_cache_creation_1h: None,
    };
    let savings = model_caching_savings(&request, &priced, usage, &database).unwrap();
    assert_eq!(savings, Decimal::ZERO);
}

#[rstest::rstest]
#[case::positive_detail_wins(Some(7), Some(99), 7)]
#[case::negative_detail_wins(Some(-2), Some(99), -2)]
#[case::negative_fallback_is_zero(Some(0), Some(-3), 0)]
#[case::absent_details_use_fallback(None, Some(5), 5)]
fn detail_counts_precede_positive_fallback(
    #[case] detail: Option<i64>,
    #[case] fallback: Option<i64>,
    #[case] expected: i64,
) {
    let counts = project_billed_token_counts(BilledTokenUsage {
        detail_reasoning: detail,
        top_level_reasoning: fallback,
        detail_cache_read: detail,
        top_level_cache_read: fallback,
        detail_cache_creation: detail,
        top_level_cache_creation: fallback,
        detail_cached_audio: None,
        detail_cache_creation_5m: None,
        detail_cache_creation_1h: None,
    });
    assert_eq!(
        (counts.reasoning, counts.cache_read, counts.cache_creation),
        (expected, expected, expected)
    );
}
