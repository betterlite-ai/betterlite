#![cfg(test)]

use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{CostCalculatorError, a2a_call_cost};

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn decimal_json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

fn object(pairs: &[(&str, Json)]) -> Json {
    let mut values = IndexMap::new();
    for (name, value) in pairs {
        values.insert((*name).to_owned(), value.clone());
    }
    Json::Object(values)
}

fn state(details: &[(&str, Json)]) -> LoggingState {
    let mut model_call_details = IndexMap::new();
    for (name, value) in details {
        model_call_details.insert((*name).to_owned(), value.clone());
    }
    let mut context = CorrelationContext::default();
    let mut logging = LoggingState::new(
        LoggingInit {
            model: "a2a_agent/agent".to_owned(),
            call_type: "a2a_send_message".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .expect("the fixture is RFC 3339"),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: Some(false),
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("the fixture builds");
    logging.model_call_details = model_call_details;
    logging
}

fn cost(details: &[(&str, Json)]) -> Result<Decimal, CostCalculatorError> {
    a2a_call_cost(Some(&state(details)))
}

fn money(text: &str) -> Decimal {
    text.parse().expect("the fixture is a decimal")
}

#[test]
fn a_missing_logging_object_is_free() {
    assert_eq!(a2a_call_cost(None), Ok(Decimal::ZERO));
}

#[test]
fn empty_model_call_details_are_free() {
    assert_eq!(cost(&[]), Ok(Decimal::ZERO));
}

#[test]
fn response_cost_wins_over_query_and_token_prices() {
    let details = [
        ("response_cost", number(5)),
        (
            "litellm_params",
            object(&[
                ("cost_per_query", number(3)),
                ("input_cost_per_token", number(1)),
                ("output_cost_per_token", number(1)),
            ]),
        ),
        (
            "usage",
            object(&[
                ("prompt_tokens", number(100)),
                ("completion_tokens", number(100)),
            ]),
        ),
    ];

    assert_eq!(cost(&details), Ok(money("5")));
}

#[test]
fn a_present_zero_response_cost_does_not_fall_through() {
    let details = [
        ("response_cost", number(0)),
        ("litellm_params", object(&[("cost_per_query", number(3))])),
    ];

    assert_eq!(cost(&details), Ok(Decimal::ZERO));
}

#[test]
fn a_null_response_cost_falls_through() {
    let details = [
        ("response_cost", Json::Null),
        ("litellm_params", object(&[("cost_per_query", number(3))])),
    ];

    assert_eq!(cost(&details), Ok(money("3")));
}

#[test]
fn cost_per_query_beats_token_rates() {
    let details = [
        (
            "litellm_params",
            object(&[
                ("cost_per_query", decimal_json("0.5")),
                ("input_cost_per_token", number(1)),
                ("output_cost_per_token", number(1)),
            ]),
        ),
        (
            "usage",
            object(&[
                ("prompt_tokens", number(100)),
                ("completion_tokens", number(100)),
            ]),
        ),
    ];

    assert_eq!(cost(&details), Ok(money("0.5")));
}

#[test]
fn a_present_zero_cost_per_query_does_not_fall_through() {
    let details = [
        (
            "litellm_params",
            object(&[
                ("cost_per_query", number(0)),
                ("input_cost_per_token", number(1)),
                ("output_cost_per_token", number(1)),
            ]),
        ),
        (
            "usage",
            object(&[
                ("prompt_tokens", number(100)),
                ("completion_tokens", number(100)),
            ]),
        ),
    ];

    assert_eq!(cost(&details), Ok(Decimal::ZERO));
}

#[test]
fn token_rates_price_the_usage_tokens() {
    let details = [
        (
            "litellm_params",
            object(&[
                ("input_cost_per_token", decimal_json("0.002")),
                ("output_cost_per_token", decimal_json("0.004")),
            ]),
        ),
        (
            "usage",
            object(&[
                ("prompt_tokens", number(100)),
                ("completion_tokens", number(50)),
            ]),
        ),
    ];

    assert_eq!(cost(&details), Ok(money("0.4")));
}

#[test]
fn a_missing_rate_contributes_zero() {
    let details = [
        (
            "litellm_params",
            object(&[("input_cost_per_token", decimal_json("0.002"))]),
        ),
        (
            "usage",
            object(&[
                ("prompt_tokens", number(100)),
                ("completion_tokens", number(50)),
            ]),
        ),
    ];

    assert_eq!(cost(&details), Ok(money("0.2")));
}

#[test]
fn token_rates_without_usage_are_free() {
    let details = [(
        "litellm_params",
        object(&[
            ("input_cost_per_token", number(1)),
            ("output_cost_per_token", number(1)),
        ]),
    )];

    assert_eq!(cost(&details), Ok(Decimal::ZERO));
}

#[test]
fn null_usage_is_free() {
    let details = [
        (
            "litellm_params",
            object(&[
                ("input_cost_per_token", number(1)),
                ("output_cost_per_token", number(1)),
            ]),
        ),
        ("usage", Json::Null),
    ];

    assert_eq!(cost(&details), Ok(Decimal::ZERO));
}

#[test]
fn a_numeric_string_price_is_accepted() {
    let details = [(
        "litellm_params",
        object(&[("cost_per_query", Json::Text("0.5".to_owned()))]),
    )];

    assert_eq!(cost(&details), Ok(money("0.5")));
}

#[test]
fn a_scientific_string_price_is_accepted() {
    let details = [(
        "litellm_params",
        object(&[("cost_per_query", Json::Text("5e-1".to_owned()))]),
    )];

    assert_eq!(cost(&details), Ok(money("0.5")));
}

#[test]
fn a_boolean_price_is_an_error() {
    let details = [("response_cost", Json::Bool(true))];

    assert_eq!(
        cost(&details),
        Err(CostCalculatorError::ParamType {
            field: "response_cost"
        })
    );
}

#[test]
fn a_text_price_is_an_error() {
    let details = [(
        "litellm_params",
        object(&[("cost_per_query", Json::Text("free".to_owned()))]),
    )];

    assert_eq!(
        cost(&details),
        Err(CostCalculatorError::ParamType {
            field: "cost_per_query"
        })
    );
}

#[test]
fn null_litellm_params_are_free() {
    let details = [("litellm_params", Json::Null)];

    assert_eq!(cost(&details), Ok(Decimal::ZERO));
}

#[test]
fn a_missing_token_count_is_zero() {
    let details = [
        (
            "litellm_params",
            object(&[("input_cost_per_token", decimal_json("0.002"))]),
        ),
        ("usage", object(&[("prompt_tokens", number(3))])),
    ];

    assert_eq!(cost(&details), Ok(money("0.006")));
}

#[test]
fn a_list_litellm_params_is_an_error() {
    let details = [("litellm_params", Json::List(vec![number(1)]))];

    assert_eq!(cost(&details), Err(CostCalculatorError::ParamsNotObject));
}

#[test]
fn a_list_usage_is_an_error() {
    let details = [
        (
            "litellm_params",
            object(&[("input_cost_per_token", number(1))]),
        ),
        ("usage", Json::List(vec![number(1)])),
    ];

    assert_eq!(cost(&details), Err(CostCalculatorError::UsageNotObject));
}

#[test]
fn a_text_token_count_is_an_error() {
    let details = [
        (
            "litellm_params",
            object(&[("input_cost_per_token", number(1))]),
        ),
        (
            "usage",
            object(&[("prompt_tokens", Json::Text("3".to_owned()))]),
        ),
    ];

    assert_eq!(
        cost(&details),
        Err(CostCalculatorError::UsageType {
            field: "usage.prompt_tokens"
        })
    );
}

#[test]
fn a_fractional_token_count_is_an_error() {
    let details = [
        (
            "litellm_params",
            object(&[("input_cost_per_token", number(1))]),
        ),
        (
            "usage",
            object(&[("completion_tokens", decimal_json("0.5"))]),
        ),
    ];

    assert_eq!(
        cost(&details),
        Err(CostCalculatorError::UsageType {
            field: "usage.completion_tokens"
        })
    );
}

#[test]
fn a_cost_per_query_does_not_validate_the_token_rates() {
    let details = [(
        "litellm_params",
        object(&[
            ("cost_per_query", number(3)),
            ("input_cost_per_token", Json::Text("free".to_owned())),
        ]),
    )];

    assert_eq!(cost(&details), Ok(money("3")));
}

#[test]
fn a_fixed_price_does_not_validate_the_usage() {
    let details = [
        ("litellm_params", object(&[("cost_per_query", number(3))])),
        ("usage", Json::List(vec![number(1)])),
    ];

    assert_eq!(cost(&details), Ok(money("3")));
}

#[test]
fn no_token_rates_does_not_validate_the_usage() {
    let details = [
        ("litellm_params", object(&[])),
        ("usage", Json::List(vec![number(1)])),
    ];

    assert_eq!(cost(&details), Ok(Decimal::ZERO));
}

#[test]
fn a_response_cost_does_not_validate_litellm_params() {
    let details = [
        ("response_cost", number(5)),
        ("litellm_params", Json::List(vec![number(1)])),
    ];

    assert_eq!(cost(&details), Ok(money("5")));
}
