#![cfg(test)]

use std::str::FromStr as _;

use pyo3::prelude::*;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::NativeCost;
use crate::cost::convert::Amount;

const SETTINGS: &str = r#"{"model_cost": {
    "gpt-test": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.001,
        "output_cost_per_token": 0.002
    }
}}"#;

const CHAT: &str = r#"{"model": "gpt-test", "custom_llm_provider": "openai",
    "response": {"kind": "chat", "model": "gpt-test",
                 "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}}"#;

// The answer completion_cost writes, in the fields these tests read.
#[derive(Debug, Deserialize)]
struct ChargeWire {
    cost: Amount,
    breakdown: Option<BreakdownWire>,
    billed_token_rates: Option<ChargeRatesWire>,
}

#[derive(Debug, Deserialize)]
struct BreakdownWire {
    #[serde(rename = "input_cost")]
    input: Option<Amount>,
    #[serde(rename = "total_cost")]
    total: Option<Amount>,
}

#[derive(Debug, Deserialize)]
struct ChargeRatesWire {
    input_cost_per_token: Amount,
}

#[derive(Debug, Deserialize)]
struct UsageAnswerWire {
    usage: Option<UsageWire>,
}

#[derive(Debug, Deserialize)]
struct UsageWire {
    total_tokens: i64,
}

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture decimal parses")
}

fn cost() -> NativeCost {
    NativeCost::new(SETTINGS).expect("the fixture settings build")
}

#[test]
fn cost_register_adds_the_native_cost_class() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        super::super::register(&module).unwrap();
        assert!(module.hasattr("NativeCost").unwrap());
    });
}

#[test]
fn cost_native_refuses_settings_text_that_is_not_json() {
    Python::initialize();
    Python::attach(|_py| {
        assert!(NativeCost::new("not json").is_err());
    });
}

#[test]
fn cost_native_prices_the_prompt_and_the_completion_apart() {
    Python::initialize();
    Python::attach(|py| {
        let text = cost()
            .cost_per_token(
                py,
                r#"{"model": "gpt-test", "prompt_tokens": 10, "completion_tokens": 5,
                    "custom_llm_provider": "openai"}"#,
            )
            .expect("the call is priced");
        let pair: Vec<f64> = serde_json::from_str(&text).expect("the pair is JSON");
        assert_eq!(pair, vec![0.01_f64, 0.01_f64]);
    });
}

#[test]
fn cost_native_completion_cost_answers_the_charge_and_its_breakdown() {
    Python::initialize();
    Python::attach(|py| {
        let text = cost()
            .completion_cost(py, CHAT)
            .expect("the response is priced");
        let answer: ChargeWire = serde_json::from_str(&text).expect("the answer is JSON");
        assert_eq!(answer.cost, Amount(dec("0.02")));
        let breakdown = answer.breakdown.expect("the breakdown is recorded");
        assert_eq!(breakdown.input, Some(Amount(dec("0.01"))));
        assert_eq!(breakdown.total, Some(Amount(dec("0.02"))));
        let rates = answer.billed_token_rates.expect("the rates are recorded");
        assert_eq!(rates.input_cost_per_token, Amount(dec("0.001")));
    });
}

#[test]
fn cost_native_response_cost_prices_the_same_request() {
    Python::initialize();
    Python::attach(|py| {
        let text = cost()
            .response_cost(py, CHAT)
            .expect("the response is priced");
        let answer: ChargeWire = serde_json::from_str(&text).expect("the answer is JSON");
        assert_eq!(answer.cost, Amount(dec("0.02")));
    });
}

#[test]
fn cost_native_usage_object_answers_the_usage_mapping() {
    Python::initialize();
    Python::attach(|py| {
        let text = NativeCost::usage_object(
            py,
            r#"{"kind": "other",
                "usage": {"prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6}}"#,
        )
        .expect("the usage reads");
        let answer: UsageAnswerWire = serde_json::from_str(&text).expect("the answer is JSON");
        let usage = answer.usage.expect("the usage comes back");
        assert_eq!(usage.total_tokens, 6);
    });
}

#[test]
fn cost_native_usage_object_answers_none_without_a_usage() {
    Python::initialize();
    Python::attach(|py| {
        let text = NativeCost::usage_object(py, "{}").expect("the empty request reads");
        let answer: UsageAnswerWire = serde_json::from_str(&text).expect("the answer is JSON");
        assert!(answer.usage.is_none());
    });
}

#[test]
fn cost_native_methods_refuse_request_text_that_is_not_json() {
    Python::initialize();
    Python::attach(|py| {
        let native = cost();
        let _per_token = native
            .cost_per_token(py, "not json")
            .expect_err("malformed text fails");
        let _completion = native
            .completion_cost(py, "not json")
            .expect_err("malformed text fails");
        let _response = native
            .response_cost(py, "not json")
            .expect_err("malformed text fails");
        let _usage = NativeCost::usage_object(py, "not json").expect_err("malformed text fails");
        let error = native
            .cost_per_token(py, "not json")
            .expect_err("malformed text fails");
        assert!(error.to_string().starts_with("ValueError:"));
    });
}
