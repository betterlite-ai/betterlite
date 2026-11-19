#![cfg(test)]

use std::str::FromStr as _;

use betterlite_anthropic::cache_aware_routing::services::ScenarioRequest;
use betterlite_anthropic::cache_aware_routing::types::CacheTokenBuckets;
use betterlite_models::price_lookup::PriceRecord;
use rstest::rstest;
use rust_decimal::Decimal;

use super::cache_cost_scenario;
use crate::cost::prompt_cache_scenarios::test_support::DEPLOYMENT_ID;
use crate::cost::prompt_cache_scenarios::test_support::Lookup;
use crate::cost::prompt_cache_scenarios::test_support::MODEL;
use crate::cost::prompt_cache_scenarios::test_support::Prices;
use crate::cost::prompt_cache_scenarios::test_support::flat_tariff;
use crate::cost::prompt_cache_scenarios::test_support::moment;
use crate::cost::prompt_cache_scenarios::test_support::number;
use crate::cost::prompt_cache_scenarios::test_support::record;
use crate::cost::prompt_cache_scenarios::test_support::tokens;
use crate::cost::prompt_cache_scenarios::test_support::without;

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).expect("a decimal literal")
}

fn scenario(tariff: Option<&PriceRecord>, tokens: &CacheTokenBuckets) -> Option<Decimal> {
    let lookup = tariff.map_or_else(
        || Lookup::new(&[]),
        |tariff| Lookup::new(&[(DEPLOYMENT_ID, tariff.clone())]),
    );
    let prices = Prices::new(lookup);
    let environment = prices.environment();
    let request = ScenarioRequest {
        model: MODEL,
        deployment_id: DEPLOYMENT_ID,
        tokens,
    };
    cache_cost_scenario(&environment, moment(), &request).map(|scenario| scenario.input_cost)
}

#[test]
fn a_scenario_prices_its_buckets_only() {
    let tokens = tokens(1000, 0, 0, 0);
    assert_eq!(scenario(Some(&flat_tariff()), &tokens), Some(dec("1")));
}

#[test]
fn a_scenario_bills_the_one_hour_rate_for_one_hour_tokens() {
    let tariff = record(&[
        ("input_cost_per_token", number("0.001")),
        ("cache_creation_input_token_cost", number("0.002")),
        ("cache_creation_input_token_cost_above_1hr", number("0.003")),
    ]);
    let tokens = tokens(0, 0, 0, 500);
    assert_eq!(scenario(Some(&tariff), &tokens), Some(dec("1.5")));
}

#[test]
fn a_scenario_without_a_tariff_is_none() {
    let tokens = tokens(1000, 0, 0, 0);
    assert_eq!(scenario(None, &tokens), None);
}

#[rstest]
#[case::input("input_cost_per_token")]
#[case::cache_read("cache_read_input_token_cost")]
fn a_scenario_without_a_required_rate_is_none(#[case] missing: &str) {
    let tariff = without(&flat_tariff(), &[missing]);
    let tokens = tokens(0, 1, 0, 0);
    assert_eq!(scenario(Some(&tariff), &tokens), None);
}

#[test]
fn a_scenario_keeps_the_buckets_it_priced() {
    let tokens = tokens(1000, 0, 0, 0);
    let prices = Prices::new(Lookup::new(&[(DEPLOYMENT_ID, flat_tariff())]));
    let environment = prices.environment();
    let request = ScenarioRequest {
        model: MODEL,
        deployment_id: DEPLOYMENT_ID,
        tokens: &tokens,
    };
    let scenario =
        cache_cost_scenario(&environment, moment(), &request).expect("a priced scenario");
    assert_eq!(scenario.tokens, tokens);
    assert_eq!(scenario.input_cost, dec("1"));
}
