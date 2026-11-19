#![cfg(test)]

use std::str::FromStr as _;

use betterlite_anthropic::cache_aware_routing::types::CacheTokenBuckets;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use rstest::rstest;
use rust_decimal::Decimal;

use super::price_cache_tokens;
use crate::cost::prompt_cache_scenarios::test_support::DEPLOYMENT_ID;
use crate::cost::prompt_cache_scenarios::test_support::Lookup;
use crate::cost::prompt_cache_scenarios::test_support::Prices;
use crate::cost::prompt_cache_scenarios::test_support::flat_tariff;
use crate::cost::prompt_cache_scenarios::test_support::moment;
use crate::cost::prompt_cache_scenarios::test_support::number;
use crate::cost::prompt_cache_scenarios::test_support::record;
use crate::cost::prompt_cache_scenarios::test_support::request;
use crate::cost::prompt_cache_scenarios::test_support::text;
use crate::cost::prompt_cache_scenarios::test_support::tokens;
use crate::cost::prompt_cache_scenarios::test_support::without;

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).expect("a decimal literal")
}

fn billed(tariff: PriceRecord, tokens: &CacheTokenBuckets, completion: i64) -> Option<Decimal> {
    let prices = Prices::new(Lookup::new(&[(DEPLOYMENT_ID, tariff)]));
    let environment = prices.environment();
    price_cache_tokens(&environment, moment(), &request(tokens, completion))
}

#[test]
fn uncached_tokens_are_billed_at_the_input_rate() {
    let tokens = tokens(1000, 0, 0, 0);
    assert_eq!(billed(flat_tariff(), &tokens, 0), Some(dec("1")));
}

#[test]
fn cache_read_tokens_are_billed_at_the_read_rate() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_read_input_token_cost", number("0.002")),
    ]);
    let tokens = tokens(0, 1000, 0, 0);
    assert_eq!(billed(tariff, &tokens, 0), Some(dec("2")));
}

#[test]
fn five_minute_creation_tokens_are_billed_at_the_creation_rate() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_creation_input_token_cost", number("0.003")),
    ]);
    let tokens = tokens(0, 0, 1000, 0);
    assert_eq!(billed(tariff, &tokens, 0), Some(dec("3")));
}

#[test]
fn one_hour_creation_tokens_are_billed_at_the_above_one_hour_rate() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_creation_input_token_cost", number("0.002")),
        ("cache_creation_input_token_cost_above_1hr", number("0.004")),
    ]);
    let tokens = tokens(0, 0, 0, 1000);
    assert_eq!(billed(tariff, &tokens, 0), Some(dec("4")));
}

#[test]
fn completion_tokens_are_billed_at_the_output_rate() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        ("output_cost_per_token", number("0.1")),
    ]);
    let tokens = tokens(0, 0, 0, 0);
    assert_eq!(billed(tariff, &tokens, 10), Some(dec("1")));
}

#[test]
fn a_mixed_request_sums_every_bucket() {
    let tokens = tokens(1, 2, 3, 4);
    assert_eq!(billed(flat_tariff(), &tokens, 5), Some(dec("0.015")));
}

#[test]
fn a_zero_rate_tariff_bills_zero() {
    let tariff = record(&[
        ("input_cost_per_token", number("0")),
        ("output_cost_per_token", number("0")),
    ]);
    let tokens = tokens(10, 0, 0, 0);
    assert_eq!(billed(tariff, &tokens, 0), Some(Decimal::ZERO));
}

#[rstest]
#[case::input("input_cost_per_token", (1, 0, 0, 0), 0)]
#[case::cache_read("cache_read_input_token_cost", (0, 1, 0, 0), 0)]
#[case::five_minute("cache_creation_input_token_cost", (0, 0, 1, 0), 0)]
#[case::one_hour("cache_creation_input_token_cost_above_1hr", (0, 0, 0, 1), 0)]
#[case::output("output_cost_per_token", (0, 0, 0, 0), 1)]
fn a_tariff_without_a_required_rate_is_refused(
    #[case] missing: &str,
    #[case] counts: (i64, i64, i64, i64),
    #[case] completion: i64,
) {
    let tariff = without(&flat_tariff(), &[missing]);
    let tokens = tokens(counts.0, counts.1, counts.2, counts.3);
    assert_eq!(billed(tariff, &tokens, completion), None);
}

#[rstest]
#[case::text(text("0.001"))]
#[case::boolean(Scalar::Bool(true))]
#[case::negative(number("-0.001"))]
#[case::null(Scalar::Null)]
fn an_unusable_input_rate_is_refused(#[case] input: Scalar) {
    let tariff = record(&[("input_cost_per_token", input)]);
    let tokens = tokens(1, 0, 0, 0);
    assert_eq!(billed(tariff, &tokens, 0), None);
}

#[test]
fn an_unusable_above_variant_of_a_required_rate_is_refused() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_read_input_token_cost", number("0.001")),
        (
            "cache_read_input_token_cost_above_200k_tokens",
            text("free"),
        ),
    ]);
    let tokens = tokens(0, 1, 0, 0);
    assert_eq!(billed(tariff, &tokens, 0), None);
}

#[test]
fn a_null_above_variant_is_ignored() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_read_input_token_cost", number("0.001")),
        (
            "cache_read_input_token_cost_above_200k_tokens",
            Scalar::Null,
        ),
    ]);
    let tokens = tokens(0, 1, 0, 0);
    assert_eq!(billed(tariff, &tokens, 0), Some(dec("0.001")));
}

#[test]
fn a_tariff_without_a_non_required_rate_is_accepted() {
    let tariff = record(&[("input_cost_per_token", number("0.001"))]);
    let tokens = tokens(10, 0, 0, 0);
    assert_eq!(billed(tariff, &tokens, 0), Some(dec("0.01")));
}

#[test]
fn a_negative_completion_count_has_no_price() {
    let tokens = tokens(10, 0, 0, 0);
    assert_eq!(billed(flat_tariff(), &tokens, -1), None);
}

#[test]
fn a_deployment_without_a_tariff_and_an_unknown_model_has_no_price() {
    let tokens = tokens(10, 0, 0, 0);
    let prices = Prices::new(Lookup::new(&[]));
    let environment = prices.environment();
    assert!(
        price_cache_tokens(&environment, moment(), &request(&tokens, 0)).is_none(),
        "no tariff means no price"
    );
}
