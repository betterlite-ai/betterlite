#![cfg(test)]

use std::slice;
use std::str::FromStr as _;

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;
use rust_decimal::Decimal;

use super::base_cost::get_token_base_cost;
use super::batch::get_batch_cost_rates;
use super::deployment::{calculate_cost_component, deployment_pricing};
use super::*;

fn model(entries: &[(&str, &str)]) -> ModelInfo {
    entries
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                serde_json::from_str(value).expect("a fixture scalar parses"),
            )
        })
        .collect()
}

fn moment() -> DateTime<Utc> {
    DateTime::from_str("2024-01-01T00:00:00Z").expect("a fixture moment parses")
}

fn context<'fixture>(
    service_tier: Option<&'fixture str>,
    provider: Option<&'fixture str>,
    database: &'fixture TimeZoneDatabase,
) -> RequestContext<'fixture> {
    RequestContext {
        service_tier,
        provider,
        moment: moment(),
        database,
    }
}

fn usage(prompt_tokens: i64) -> Usage {
    Usage { prompt_tokens }
}

#[rstest]
#[case::fast_maps_to_priority("fast", "priority")]
#[case::priority_keeps_priority("priority", "priority")]
#[case::flex_keeps_flex("flex", "flex")]
#[case::balanced_keeps_balanced("balanced", "balanced")]
#[case::ultrafast_keeps_ultrafast("ultrafast", "ultrafast")]
#[case::unknown_tier_is_standard("auto", "")]
fn service_tier_matching_is_case_insensitive(#[case] tier: &str, #[case] suffix: &str) {
    let upper = tier.to_ascii_uppercase();
    let expected = if suffix.is_empty() {
        "input_cost_per_token".to_owned()
    } else {
        format!("input_cost_per_token_{suffix}")
    };
    assert_eq!(
        rates::service_tier_cost_key("input_cost_per_token", Some(&upper)),
        expected
    );
}

#[test]
fn fast_priority_price_wins_over_standard() {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[
        ("input_cost_per_token", "\"0.1\""),
        ("input_cost_per_token_priority", "\"0.2\""),
        ("output_cost_per_token_priority", "\"bad\""),
    ]);
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: &[],
        off_peak_pricing: None,
    };
    let rates = get_token_base_cost(&pricing, usage(1), &context(Some("fast"), None, &database))
        .expect("a valid service-tier price resolves");
    assert_eq!(rates.input_rate, Decimal::new(2, 1));
    assert_eq!(rates.output_rate, Decimal::ZERO);
}

#[test]
fn invalid_nonnull_rate_does_not_take_missing_key_fallback() {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[
        ("input_cost_per_token", "\"0.4\""),
        ("input_cost_per_token_priority", "\"not-a-number\""),
    ]);
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: &[],
        off_peak_pricing: None,
    };
    let rates = get_token_base_cost(
        &pricing,
        usage(1),
        &context(Some("priority"), None, &database),
    )
    .expect("a malformed string defaults instead of erroring");
    assert_eq!(rates.input_rate, Decimal::ZERO);
}

#[rstest]
#[case::openai_strict_boundary_not_crossed(100, false)]
#[case::openai_strict_boundary_crossed(101, true)]
#[case::xai_inclusive_boundary_crossed(100, true)]
fn threshold_boundary_matches_provider_rule(#[case] prompt_tokens: i64, #[case] crosses: bool) {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[
        ("input_cost_per_token", "\"0.1\""),
        ("input_cost_per_token_above_100_tokens", "\"0.5\""),
    ]);
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: &[],
        off_peak_pricing: None,
    };
    let provider = (prompt_tokens == 100 && crosses).then_some("xai");
    let rates = get_token_base_cost(
        &pricing,
        usage(prompt_tokens),
        &context(None, provider, &database),
    )
    .expect("a well-formed threshold resolves");
    let expected = if crosses {
        Decimal::new(5, 1)
    } else {
        Decimal::new(1, 1)
    };
    assert_eq!(rates.input_rate, expected);
}

#[test]
fn malformed_threshold_sort_key_is_a_typed_error() {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[
        ("input_cost_per_token", "\"0.1\""),
        ("input_cost_per_token_above_notanumber_tokens", "\"0.5\""),
    ]);
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: &[],
        off_peak_pricing: None,
    };
    get_token_base_cost(&pricing, usage(1), &context(None, None, &database)).unwrap_err();
}

#[test]
fn missing_cache_rates_inherit_resolved_input_rate() {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[("input_cost_per_token", "\"0.2\"")]);
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: &[],
        off_peak_pricing: None,
    };
    let rates = get_token_base_cost(&pricing, usage(1), &context(None, None, &database))
        .expect("an absent cache price resolves");
    assert_eq!(rates.cache_read_rate, Decimal::new(2, 1));
    assert_eq!(rates.cache_creation_rate, Decimal::new(2, 1));
    assert_eq!(rates.cache_creation_above_1hr_rate, Decimal::new(2, 1));
}

#[test]
fn explicit_zero_cache_price_survives() {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[
        ("input_cost_per_token", "\"0.2\""),
        ("cache_read_input_token_cost", "0"),
    ]);
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: &[],
        off_peak_pricing: None,
    };
    let rates = get_token_base_cost(&pricing, usage(1), &context(None, None, &database))
        .expect("an explicit zero resolves");
    assert_eq!(rates.cache_read_rate, Decimal::ZERO);
}

#[test]
fn tiered_table_output_falls_back_to_model_output_rate() {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[("output_cost_per_token", "\"0.3\"")]);
    let tier: Tier = serde_json::from_str(r#"{"range":[0,1000],"input_cost_per_token":"0.1"}"#)
        .expect("a fixture tier parses");
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: slice::from_ref(&tier),
        off_peak_pricing: None,
    };
    let rates = get_token_base_cost(&pricing, usage(1), &context(None, None, &database))
        .expect("a tier missing output falls back to the model rate");
    assert_eq!(rates.output_rate, Decimal::new(3, 1));
}

#[test]
fn tiered_one_hour_zero_falls_back_to_cache_creation() {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[]);
    let tier: Tier = serde_json::from_str(
        r#"{"range":[0,1000],"input_cost_per_token":"0.1","cache_creation_input_token_cost":"0.2","cache_creation_input_token_cost_above_1hr":0}"#,
    )
    .expect("a fixture tier parses");
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: slice::from_ref(&tier),
        off_peak_pricing: None,
    };
    let rates = get_token_base_cost(&pricing, usage(1), &context(None, None, &database))
        .expect("a zero one-hour rate resolves");
    assert_eq!(rates.cache_creation_above_1hr_rate, Decimal::new(2, 1));
}

#[test]
fn priced_tier_requires_an_input_rate_key() {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[("input_cost_per_token", "\"0.1\"")]);
    let tier: Tier = serde_json::from_str(r#"{"range":[0,1000],"output_cost_per_token":"0.2"}"#)
        .expect("a fixture tier parses");
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: slice::from_ref(&tier),
        off_peak_pricing: None,
    };
    let rates = get_token_base_cost(&pricing, usage(1), &context(None, None, &database))
        .expect("a tier without an input rate is skipped entirely");
    assert_eq!(rates.input_rate, Decimal::new(1, 1));
}

#[test]
fn reasoning_prefers_service_output_price_over_standard_reasoning() {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[
        ("output_cost_per_token", "\"0.3\""),
        ("output_cost_per_token_priority", "\"0.4\""),
        ("output_cost_per_reasoning_token", "\"0.9\""),
    ]);
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: &[],
        off_peak_pricing: None,
    };
    let rates = get_token_base_cost(
        &pricing,
        usage(1),
        &context(Some("priority"), None, &database),
    )
    .expect("a service output price suppresses the standard reasoning rate");
    assert_eq!(rates.reasoning_rate, rates.output_rate);
}

#[test]
fn reasoning_falls_back_to_standard_reasoning_rate() {
    let database = TimeZoneDatabase::from_env();
    let prices = model(&[
        ("output_cost_per_token", "\"0.3\""),
        ("output_cost_per_reasoning_token", "\"0.9\""),
    ]);
    let pricing = ModelPricing {
        model: &prices,
        tiered_pricing: &[],
        off_peak_pricing: None,
    };
    let rates = get_token_base_cost(&pricing, usage(1), &context(None, None, &database))
        .expect("a standard reasoning rate resolves");
    assert_eq!(rates.reasoning_rate, Decimal::new(9, 1));
}

#[test]
fn batch_families_select_independent_thresholds() {
    let prices = model(&[
        ("input_cost_per_token_batches", "\"1\""),
        ("input_cost_per_token_above_1k_tokens_batches", "\"2\""),
        ("output_cost_per_token_batches", "\"3\""),
    ]);
    let rates = get_batch_cost_rates(&prices, usage(2_000), None).expect("batch rates resolve");
    assert_eq!(rates.input, Some(Decimal::from(2_u8)));
    assert_eq!(rates.output, Some(Decimal::from(3_u8)));
}

#[test]
fn batch_tier_falls_back_to_flat_rate_when_tier_rate_is_malformed() {
    let prices = model(&[
        ("input_cost_per_token_batches", "\"1\""),
        (
            "input_cost_per_token_above_1k_tokens_batches",
            "\"garbage\"",
        ),
    ]);
    let rates = get_batch_cost_rates(&prices, usage(2_000), None).expect("batch rates resolve");
    assert_eq!(rates.input, Some(Decimal::from(1_u8)));
}

#[test]
fn batch_does_not_fall_back_to_a_lower_threshold() {
    let prices = model(&[
        ("input_cost_per_token_above_1k_tokens_batches", "\"2\""),
        (
            "input_cost_per_token_above_5k_tokens_batches",
            "\"garbage\"",
        ),
    ]);
    let rates = get_batch_cost_rates(&prices, usage(10_000), None).expect("batch rates resolve");
    assert_eq!(rates.input, None);
}

#[test]
fn component_only_multiplies_positive_usage() {
    let prices = model(&[("input_cost_per_token", "\"0.2\"")]);
    assert_eq!(
        calculate_cost_component(&prices, "input_cost_per_token", Some(Decimal::from(-1_i32)))
            .unwrap(),
        Decimal::ZERO
    );
    assert_eq!(
        calculate_cost_component(&prices, "input_cost_per_token", Some(Decimal::from(3_i32)))
            .unwrap(),
        Decimal::new(6, 1)
    );
}

#[test]
fn deployment_keeps_explicit_zero_and_drops_nested_tables() {
    let prices = model(&[
        ("input_cost_per_token", "0"),
        ("tiered_pricing", "[1]"),
        ("unrelated", "4"),
    ]);
    let selected = deployment_pricing(Some(&prices)).unwrap().unwrap();
    assert_eq!(selected.get("input_cost_per_token"), Some(&Decimal::ZERO));
    assert_eq!(selected.len(), 1);
}

#[test]
fn deployment_is_none_when_nothing_parses() {
    let prices = model(&[("tiered_pricing", "[1]")]);
    assert_eq!(deployment_pricing(Some(&prices)).unwrap(), None);
}

#[test]
fn nonfinite_string_rate_is_a_typed_error() {
    let prices = model(&[("input_cost_per_token", "\"nan\"")]);
    calculate_cost_component(&prices, "input_cost_per_token", Some(Decimal::ONE)).unwrap_err();
}

#[test]
fn underscored_numeric_string_rate_parses() {
    let prices = model(&[("input_cost_per_token", "\"1_000\"")]);
    assert_eq!(
        calculate_cost_component(&prices, "input_cost_per_token", Some(Decimal::ONE)).unwrap(),
        Decimal::from(1_000_u16)
    );
}
