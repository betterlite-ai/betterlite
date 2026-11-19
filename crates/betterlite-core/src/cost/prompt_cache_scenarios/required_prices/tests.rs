#![cfg(test)]

use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;

use super::has_required_prices;
use crate::cost::prompt_cache_scenarios::test_support::number;
use crate::cost::prompt_cache_scenarios::test_support::record;
use crate::cost::prompt_cache_scenarios::test_support::text;
use crate::cost::prompt_cache_scenarios::test_support::tokens;

fn input_only() -> PriceRecord {
    record(&[("input_cost_per_token", number("0.001"))])
}

#[test]
fn an_empty_tariff_has_no_input_price() {
    assert!(!has_required_prices(&record(&[]), &tokens(0, 0, 0, 0), 0));
}

#[test]
fn the_input_rate_is_always_required() {
    assert!(has_required_prices(&input_only(), &tokens(0, 0, 0, 0), 0));
    assert!(has_required_prices(&input_only(), &tokens(100, 0, 0, 0), 5));
}

#[test]
fn a_read_count_requires_the_read_rate() {
    assert!(!has_required_prices(&input_only(), &tokens(0, 1, 0, 0), 0));
    let with_read = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_read_input_token_cost", number("0.0001")),
    ]);
    assert!(has_required_prices(&with_read, &tokens(0, 1, 0, 0), 0));
}

#[test]
fn a_five_minute_creation_count_requires_the_creation_rate() {
    assert!(!has_required_prices(&input_only(), &tokens(0, 0, 1, 0), 0));
    let with_creation = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_creation_input_token_cost", number("0.002")),
    ]);
    assert!(has_required_prices(&with_creation, &tokens(0, 0, 1, 0), 0));
}

#[test]
fn a_one_hour_creation_count_requires_the_above_one_hour_rate() {
    let with_creation = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_creation_input_token_cost", number("0.002")),
    ]);
    assert!(!has_required_prices(&with_creation, &tokens(0, 0, 0, 1), 0));
    let with_above = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_creation_input_token_cost_above_1hr", number("0.004")),
    ]);
    assert!(has_required_prices(&with_above, &tokens(0, 0, 0, 1), 0));
}

#[test]
fn a_completion_count_requires_the_output_rate() {
    assert!(!has_required_prices(&input_only(), &tokens(0, 0, 0, 0), 1));
    assert!(has_required_prices(&input_only(), &tokens(0, 0, 0, 0), 0));
    let with_output = record(&[
        ("input_cost_per_token", number("0.001")),
        ("output_cost_per_token", number("0.002")),
    ]);
    assert!(has_required_prices(&with_output, &tokens(0, 0, 0, 0), 1));
}

#[test]
fn a_zero_count_requires_nothing_beyond_the_input_rate() {
    assert!(has_required_prices(&input_only(), &tokens(10, 0, 0, 0), 0));
}

#[test]
fn an_unusable_required_rate_is_refused() {
    let text_rate = record(&[("input_cost_per_token", text("0.001"))]);
    assert!(!has_required_prices(&text_rate, &tokens(1, 0, 0, 0), 0));
    let negative_rate = record(&[("input_cost_per_token", number("-0.001"))]);
    assert!(!has_required_prices(&negative_rate, &tokens(1, 0, 0, 0), 0));
}

#[test]
fn an_unusable_above_variant_of_a_required_rate_is_refused() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_read_input_token_cost", number("0.0001")),
        (
            "cache_read_input_token_cost_above_200k_tokens",
            text("free"),
        ),
    ]);
    assert!(!has_required_prices(&tariff, &tokens(0, 1, 0, 0), 0));
}

#[test]
fn an_unusable_above_variant_of_the_input_rate_is_refused() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        ("input_cost_per_token_above_200k_tokens", text("free")),
    ]);
    assert!(!has_required_prices(&tariff, &tokens(1, 0, 0, 0), 0));
}

#[test]
fn an_above_variant_of_an_unrequired_rate_is_ignored() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        (
            "cache_read_input_token_cost_above_200k_tokens",
            text("free"),
        ),
    ]);
    assert!(has_required_prices(&tariff, &tokens(1, 0, 0, 0), 0));
}

#[test]
fn a_null_above_variant_is_ignored() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_read_input_token_cost", number("0.0001")),
        (
            "cache_read_input_token_cost_above_200k_tokens",
            Scalar::Null,
        ),
    ]);
    assert!(has_required_prices(&tariff, &tokens(0, 1, 0, 0), 0));
}
