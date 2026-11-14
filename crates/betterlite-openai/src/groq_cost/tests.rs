#![cfg(test)]

use betterlite_models::price_fields::nested::SearchContextPricing;
use rstest::rstest;
use rust_decimal::Decimal;

use super::{ServerToolUse, web_search_cost};

fn pricing(medium: Option<Decimal>) -> SearchContextPricing {
    SearchContextPricing {
        search_context_size_low: None,
        search_context_size_medium: medium,
        search_context_size_high: None,
    }
}

fn usage(searches: Option<u64>, opens: Option<u64>) -> ServerToolUse {
    ServerToolUse {
        web_search_requests: searches,
        browser_open_requests: opens,
    }
}

fn cost(usage: Option<&ServerToolUse>, pricing: Option<&SearchContextPricing>) -> Decimal {
    web_search_cost(usage, pricing).expect("a cost")
}

#[test]
fn no_server_tool_use_costs_nothing() {
    assert_eq!(
        cost(None, Some(&pricing(Some(Decimal::new(5, 3))))),
        Decimal::ZERO
    );
}

#[test]
fn null_counts_cost_nothing() {
    assert_eq!(
        cost(
            Some(&usage(None, None)),
            Some(&pricing(Some(Decimal::new(5, 3))))
        ),
        Decimal::ZERO
    );
}

#[test]
fn searches_are_charged_at_the_medium_search_rate() {
    assert_eq!(
        cost(
            Some(&usage(Some(3), None)),
            Some(&pricing(Some(Decimal::new(5, 3))))
        ),
        Decimal::new(15, 3)
    );
}

#[test]
fn browser_opens_are_charged_one_thousandth_each() {
    assert_eq!(cost(Some(&usage(None, Some(4))), None), Decimal::new(4, 3));
}

#[test]
fn searches_and_opens_add_up() {
    assert_eq!(
        cost(
            Some(&usage(Some(2), Some(3))),
            Some(&pricing(Some(Decimal::new(5, 3))))
        ),
        Decimal::new(13, 3)
    );
}

#[test]
fn absent_pricing_charges_only_the_browser_visits() {
    assert_eq!(
        cost(Some(&usage(Some(2), Some(1))), None),
        Decimal::new(1, 3)
    );
}

#[test]
fn pricing_without_a_medium_rate_charges_only_the_browser_visits() {
    assert_eq!(
        cost(Some(&usage(Some(2), Some(1))), Some(&pricing(None))),
        Decimal::new(1, 3)
    );
}

#[test]
fn a_zero_medium_rate_is_used_as_given() {
    assert_eq!(
        cost(
            Some(&usage(Some(2), None)),
            Some(&pricing(Some(Decimal::ZERO)))
        ),
        Decimal::ZERO
    );
}

#[rstest]
#[case::one_search(1)]
#[case::ten_searches(10)]
#[case::many_searches(100)]
fn each_search_adds_the_medium_rate(#[case] searches: u64) {
    let expected = Decimal::new(5, 3)
        .checked_mul(Decimal::from(searches))
        .expect("a product");
    assert_eq!(
        cost(
            Some(&usage(Some(searches), None)),
            Some(&pricing(Some(Decimal::new(5, 3))))
        ),
        expected
    );
}
