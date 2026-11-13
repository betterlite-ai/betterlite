#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::error::ProtocolCostError;
use super::{A2aCall, A2aUsage, McpCall, ProtocolCall, protocol_charge};
use crate::decisions::Json;

fn json(document: &str) -> Json {
    serde_json::from_str(document).expect("the fixture is JSON")
}

fn money(text: &str) -> Decimal {
    text.parse().expect("the fixture is a decimal")
}

fn agent(
    response_cost: &str,
    cost_per_query: &str,
    input_rate: &str,
    output_rate: &str,
    usage: A2aUsage,
) -> ProtocolCall {
    ProtocolCall::A2a(A2aCall {
        response_cost: json(response_cost),
        cost_per_query: json(cost_per_query),
        input_cost_per_token: json(input_rate),
        output_cost_per_token: json(output_rate),
        usage,
    })
}

fn tokens(prompt_tokens: i64, completion_tokens: i64) -> A2aUsage {
    A2aUsage::Tokens {
        prompt_tokens,
        completion_tokens,
    }
}

fn tool(response_cost: &str, name: &str, default_cost: &str, by_tool: &str) -> ProtocolCall {
    ProtocolCall::Mcp(McpCall {
        response_cost: json(response_cost),
        tool_name: name.to_owned(),
        default_cost_per_query: json(default_cost),
        tool_name_to_cost_per_query: serde_json::from_str(by_tool).expect("the fixture is a map"),
    })
}

#[rstest]
#[case::response_cost_beats_everything(
    agent("0.5", "0.25", "0.001", "0.002", tokens(10, 5)),
    "0.5"
)]
#[case::an_explicit_zero_response_cost_still_wins(
    agent("0", "0.25", "0.001", "0.002", tokens(10, 5)),
    "0"
)]
#[case::cost_per_query_beats_token_rates(
    agent("null", "0.25", "0.001", "0.002", tokens(10, 5)),
    "0.25"
)]
#[case::an_explicit_zero_query_cost_still_wins(
    agent("null", "0", "0.001", "0.002", tokens(10, 5)),
    "0"
)]
#[case::both_token_rates(agent("null", "null", "0.001", "0.002", tokens(10, 5)), "0.02")]
#[case::only_the_input_rate(agent("null", "null", "0.001", "null", tokens(10, 5)), "0.01")]
#[case::only_the_output_rate(agent("null", "null", "null", "0.002", tokens(10, 5)), "0.01")]
#[case::a_zero_rate_contributes_zero(agent("null", "null", "0", "0.002", tokens(10, 5)), "0.01")]
#[case::no_usage_means_no_token_cost(
    agent("null", "null", "0.001", "0.002", A2aUsage::Absent),
    "0"
)]
#[case::a_dictionary_usage_has_no_token_attributes(
    agent("null", "null", "0.001", "0.002", A2aUsage::Mapping),
    "0"
)]
#[case::no_parameters_is_free(agent("null", "null", "null", "null", tokens(10, 5)), "0")]
#[case::numeric_strings_are_floats(
    agent("null", "\" 0.5 \"", "null", "null", A2aUsage::Absent),
    "0.5"
)]
#[case::underscored_strings_are_floats(
    agent("null", "\"1_0\"", "null", "null", A2aUsage::Absent),
    "10"
)]
#[case::exponent_strings_are_floats(
    agent("null", "\"1e-2\"", "null", "null", A2aUsage::Absent),
    "0.01"
)]
#[case::rate_strings_are_floats(
    agent("null", "null", "\"0.001\"", "\"2e-3\"", tokens(10, 5)),
    "0.02"
)]
#[case::a_negative_response_cost_is_kept(
    agent("-1.5", "null", "null", "null", A2aUsage::Absent),
    "-1.5"
)]
fn agent_charge_follows_the_precedence(#[case] call: ProtocolCall, #[case] expected: &str) {
    assert_eq!(protocol_charge(&call).unwrap(), money(expected));
}

#[rstest]
#[case::words(
    agent("null", "\"abc\"", "null", "null", A2aUsage::Absent),
    "cost_per_query"
)]
#[case::not_a_number(
    agent("null", "\"nan\"", "null", "null", A2aUsage::Absent),
    "cost_per_query"
)]
#[case::infinity(
    agent("null", "\"inf\"", "null", "null", A2aUsage::Absent),
    "cost_per_query"
)]
#[case::empty_text(
    agent("null", "\"\"", "null", "null", A2aUsage::Absent),
    "cost_per_query"
)]
#[case::a_leading_underscore(
    agent("null", "\"_1\"", "null", "null", A2aUsage::Absent),
    "cost_per_query"
)]
#[case::a_doubled_underscore(
    agent("null", "\"1__0\"", "null", "null", A2aUsage::Absent),
    "cost_per_query"
)]
#[case::a_boolean(
    agent("null", "true", "null", "null", A2aUsage::Absent),
    "cost_per_query"
)]
#[case::a_list(
    agent("null", "[1]", "null", "null", A2aUsage::Absent),
    "cost_per_query"
)]
#[case::an_object(
    agent("null", "{}", "null", "null", A2aUsage::Absent),
    "cost_per_query"
)]
#[case::a_bad_response_cost(
    agent("\"x\"", "null", "null", "null", A2aUsage::Absent),
    "response_cost"
)]
#[case::a_bad_input_rate(
    agent("null", "null", "\"x\"", "null", tokens(1, 1)),
    "input_cost_per_token"
)]
#[case::a_bad_output_rate(
    agent("null", "null", "null", "\"x\"", tokens(1, 1)),
    "output_cost_per_token"
)]
#[case::a_bad_rate_with_dictionary_usage(
    agent("null", "null", "\"x\"", "null", A2aUsage::Mapping),
    "input_cost_per_token"
)]
#[case::a_null_tool_price(
    tool("null", "search", "0.1", r#"{"search": null}"#),
    "tool_name_to_cost_per_query"
)]
#[case::a_text_tool_price(
    tool("null", "search", "0.1", r#"{"search": "0.2"}"#),
    "tool_name_to_cost_per_query"
)]
#[case::a_boolean_tool_price(
    tool("null", "search", "0.1", r#"{"search": true}"#),
    "tool_name_to_cost_per_query"
)]
#[case::a_text_default(tool("null", "fetch", "\"0.1\"", "{}"), "default_cost_per_query")]
#[case::a_list_default(tool("null", "fetch", "[]", "{}"), "default_cost_per_query")]
#[case::a_text_response_cost(tool("\"0.5\"", "fetch", "null", "{}"), "response_cost")]
fn protocol_charge_rejects_values_that_are_not_finite_numbers(
    #[case] call: ProtocolCall,
    #[case] field: &str,
) {
    assert!(matches!(
        protocol_charge(&call),
        Err(ProtocolCostError::InvalidMoney { field: named }) if named == field
    ));
}

#[test]
fn agent_charge_ignores_a_bad_rate_when_there_is_no_usage() {
    let call = agent("null", "null", "\"x\"", "null", A2aUsage::Absent);
    assert_eq!(protocol_charge(&call).unwrap(), Decimal::ZERO);
}

#[test]
fn agent_charge_reports_a_product_overflow() {
    let call = agent(
        "null",
        "null",
        "100000000000000000000",
        "null",
        tokens(i64::MAX, 0),
    );
    assert!(matches!(
        protocol_charge(&call),
        Err(ProtocolCostError::Overflow)
    ));
}

#[test]
fn agent_charge_reports_a_sum_overflow() {
    let rate = "50000000000000000000";
    let call = agent("null", "null", rate, rate, tokens(i64::MAX, i64::MAX));
    assert!(matches!(
        protocol_charge(&call),
        Err(ProtocolCostError::Overflow)
    ));
}

#[rstest]
#[case::response_cost_beats_everything(tool("0.5", "search", "0.1", r#"{"search": 0.2}"#), "0.5")]
#[case::an_explicit_zero_response_cost_wins(tool("0", "search", "0.1", r#"{"search": 0.2}"#), "0")]
#[case::the_tool_price_beats_the_default(
    tool("null", "search", "0.1", r#"{"search": 0.2}"#),
    "0.2"
)]
#[case::a_zero_tool_price_beats_the_default(tool("null", "search", "0.1", r#"{"search": 0}"#), "0")]
#[case::another_tool_falls_to_the_default(
    tool("null", "fetch", "0.1", r#"{"search": 0.2}"#),
    "0.1"
)]
#[case::a_zero_default_is_used(tool("null", "fetch", "0", "{}"), "0")]
#[case::nothing_set_is_free(tool("null", "fetch", "null", "{}"), "0")]
#[case::tool_names_match_exactly(tool("null", "Search", "null", r#"{"search": 0.2}"#), "0")]
#[case::a_bad_default_is_not_read_when_the_tool_has_a_price(
    tool("null", "search", "\"x\"", r#"{"search": 0.2}"#),
    "0.2"
)]
fn tool_charge_follows_the_precedence(#[case] call: ProtocolCall, #[case] expected: &str) {
    assert_eq!(protocol_charge(&call).unwrap(), money(expected));
}
