#![cfg(test)]

use std::str::FromStr as _;
use std::sync::Mutex;

use betterlite_openai::chat::response::ChatResponse;
use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use super::{CostCalculator, CostError, CustomRates, completion_charge};
use crate::lens_inference::quote::{Deployment, DeploymentParams};

struct Fixed(Decimal);

impl CostCalculator for Fixed {
    fn completion_cost(
        &self,
        _response: &ChatResponse,
        _custom: Option<&CustomRates<'_>>,
    ) -> Result<Decimal, CostError> {
        Ok(self.0)
    }
}

struct Recording {
    amount: Decimal,
    seen: Mutex<Option<(String, Decimal, Decimal)>>,
}

impl CostCalculator for Recording {
    fn completion_cost(
        &self,
        _response: &ChatResponse,
        custom: Option<&CustomRates<'_>>,
    ) -> Result<Decimal, CostError> {
        let seen = custom.map(|rates| {
            (
                rates.model.to_owned(),
                rates.input_cost_per_token,
                rates.output_cost_per_token,
            )
        });
        *lock(&self.seen) = seen;
        Ok(self.amount)
    }
}

struct Failing;

impl CostCalculator for Failing {
    fn completion_cost(
        &self,
        _response: &ChatResponse,
        _custom: Option<&CustomRates<'_>>,
    ) -> Result<Decimal, CostError> {
        Err(CostError::Unavailable)
    }
}

fn amount(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn response(prompt: u64, completion: u64) -> ChatResponse {
    let total = prompt.checked_add(completion).unwrap();
    let prompt_text = prompt.to_string();
    let completion_text = completion.to_string();
    let total_text = total.to_string();
    let json = [
        "{\"id\":\"x\",\"object\":\"chat.completion\",\"created\":1,\"model\":\"lens-model\",",
        "\"choices\":[],\"usage\":{\"prompt_tokens\":",
        prompt_text.as_str(),
        ",\"completion_tokens\":",
        completion_text.as_str(),
        ",\"total_tokens\":",
        total_text.as_str(),
        "}}",
    ]
    .concat();
    serde_json::from_str(&json).unwrap()
}

fn deployment(input: Option<Decimal>, output: Option<Decimal>) -> Deployment {
    Deployment {
        litellm_params: DeploymentParams {
            model: "lens-model".to_owned(),
            input_cost_per_token: input,
            output_cost_per_token: output,
        },
    }
}

#[test]
fn one_deployment_with_both_custom_rates_prices_with_those_rates() {
    let calculator = Recording {
        amount: amount("2.5"),
        seen: Mutex::new(None),
    };
    let deployments = [deployment(Some(amount("0.25")), Some(amount("0.5")))];
    let cost =
        completion_charge(&deployments, &response(2, 4), amount("100"), &calculator).unwrap();
    assert_eq!(cost, amount("2.5"));
    let seen = lock(&calculator.seen).clone();
    assert_eq!(
        seen,
        Some(("lens-model".to_owned(), amount("0.25"), amount("0.5")))
    );
}

#[test]
fn a_zero_custom_charge_is_a_real_cost() {
    let deployments = [deployment(Some(amount("0.25")), Some(amount("0.5")))];
    let cost = completion_charge(
        &deployments,
        &response(0, 0),
        amount("100"),
        &Fixed(Decimal::ZERO),
    )
    .unwrap();
    assert_eq!(cost, Decimal::ZERO);
}

#[test]
fn an_unavailable_custom_charge_is_an_error() {
    let deployments = [deployment(Some(amount("0.25")), Some(amount("0.5")))];
    let error =
        completion_charge(&deployments, &response(2, 4), amount("100"), &Failing).unwrap_err();
    assert!(matches!(error, CostError::Unavailable));
}

#[test]
fn two_deployments_use_the_provider_cost() {
    let calculator = Recording {
        amount: amount("7.5"),
        seen: Mutex::new(Some(("stale".to_owned(), Decimal::ZERO, Decimal::ZERO))),
    };
    let deployments = [
        deployment(Some(amount("0.25")), Some(amount("0.5"))),
        deployment(Some(amount("0.25")), Some(amount("0.5"))),
    ];
    let cost =
        completion_charge(&deployments, &response(2, 4), amount("100"), &calculator).unwrap();
    assert_eq!(cost, amount("7.5"));
    assert_eq!(*lock(&calculator.seen), None);
}

#[test]
fn one_custom_rate_uses_the_provider_cost() {
    let deployments = [deployment(Some(amount("0.25")), None)];
    let cost = completion_charge(
        &deployments,
        &response(2, 4),
        amount("100"),
        &Fixed(amount("7.5")),
    )
    .unwrap();
    assert_eq!(cost, amount("7.5"));
}

#[test]
fn no_deployments_use_the_provider_cost() {
    let cost =
        completion_charge(&[], &response(2, 4), amount("100"), &Fixed(amount("7.5"))).unwrap();
    assert_eq!(cost, amount("7.5"));
}

#[test]
fn a_non_positive_provider_cost_falls_back_to_the_estimate() {
    let deployments = [deployment(None, None)];
    let zero = completion_charge(
        &deployments,
        &response(2, 4),
        amount("100"),
        &Fixed(Decimal::ZERO),
    )
    .unwrap();
    assert_eq!(zero, amount("100"));
    let negative = completion_charge(
        &deployments,
        &response(2, 4),
        amount("100"),
        &Fixed(amount("-1")),
    )
    .unwrap();
    assert_eq!(negative, amount("100"));
}

#[test]
fn an_unavailable_provider_cost_is_an_error() {
    let deployments = [deployment(None, None)];
    let error =
        completion_charge(&deployments, &response(2, 4), amount("100"), &Failing).unwrap_err();
    assert!(matches!(error, CostError::Unavailable));
}
