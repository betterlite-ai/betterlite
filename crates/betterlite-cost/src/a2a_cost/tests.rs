#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{A2aCostError, A2aCostParams, A2aUsage, calculate_a2a_cost};

fn money(text: &str) -> Decimal {
    text.parse().expect("the fixture is a decimal")
}

fn usage(prompt_tokens: u64, completion_tokens: u64) -> A2aUsage {
    A2aUsage {
        prompt_tokens,
        completion_tokens,
    }
}

fn params(
    response_cost: Option<Decimal>,
    cost_per_query: Option<Decimal>,
    input_rate: Option<Decimal>,
    output_rate: Option<Decimal>,
) -> A2aCostParams {
    A2aCostParams {
        response_cost,
        cost_per_query,
        input_cost_per_token: input_rate,
        output_cost_per_token: output_rate,
    }
}

#[rstest]
#[case::response_cost_beats_query_and_rates(
    params(Some(money("5")), Some(money("3")), Some(money("1")), Some(money("1"))),
    Some(usage(100, 100)),
    money("5")
)]
#[case::response_cost_zero_stays_present(
    params(
        Some(Decimal::ZERO),
        Some(money("3")),
        Some(money("1")),
        Some(money("1"))
    ),
    Some(usage(100, 100)),
    money("0")
)]
#[case::cost_per_query_when_response_absent(
    params(None, Some(money("3")), Some(money("1")), Some(money("1"))),
    Some(usage(100, 100)),
    money("3")
)]
#[case::cost_per_query_zero_stays_present(
    params(None, Some(Decimal::ZERO), Some(money("1")), Some(money("1"))),
    Some(usage(100, 100)),
    money("0")
)]
#[case::token_rates_prompt_and_completion(
    params(None, None, Some(money("0.002")), Some(money("0.004"))),
    Some(usage(100, 50)),
    money("0.4")
)]
#[case::token_rates_prompt_only(
    params(None, None, Some(money("0.002")), None),
    Some(usage(100, 50)),
    money("0.2")
)]
#[case::token_rates_completion_only(
    params(None, None, None, Some(money("0.004"))),
    Some(usage(100, 50)),
    money("0.2")
)]
#[case::token_rates_zero_counted(
    params(None, None, Some(money("0.002")), Some(money("0.004"))),
    Some(usage(0, 0)),
    money("0")
)]
#[case::token_rates_without_usage(
    params(None, None, Some(money("0.002")), Some(money("0.004"))),
    None,
    money("0")
)]
#[case::no_parameter_is_free(params(None, None, None, None), Some(usage(100, 100)), money("0"))]
#[case::null_usage_is_free(params(None, None, Some(money("1")), None), None, money("0"))]
#[case::fractional_rate(
    params(None, None, Some(money("0.000003")), None),
    Some(usage(3, 0)),
    money("0.000009")
)]
fn cost_priority_follows_litellm(
    #[case] params: A2aCostParams,
    #[case] usage: Option<A2aUsage>,
    #[case] expected: Decimal,
) {
    let cost = calculate_a2a_cost(&params, usage).expect("the fixture does not overflow");

    assert_eq!(cost, expected);
}

#[test]
fn token_cost_overflow_is_an_error() {
    let params = params(None, None, Some(Decimal::MAX), None);

    let cost = calculate_a2a_cost(&params, Some(usage(2, 0)));

    assert_eq!(cost, Err(A2aCostError::Overflow));
}

#[test]
fn token_cost_sum_overflow_is_an_error() {
    let params = params(None, None, Some(Decimal::MAX), Some(Decimal::MAX));

    let cost = calculate_a2a_cost(&params, Some(usage(1, 1)));

    assert_eq!(cost, Err(A2aCostError::Overflow));
}
