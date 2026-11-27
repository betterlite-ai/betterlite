#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_auth::reservation_cost::ModelInfoSource;
use betterlite_auth::reservation_cost::error::ModelInfoError;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;

use super::{Deployment, DeploymentParams, QuoteError, quote};

#[derive(Debug, Default)]
struct Catalog {
    entries: BTreeMap<String, JsonObject>,
}

impl ModelInfoSource for Catalog {
    fn get_model_info(&self, model: &str) -> Result<Option<JsonObject>, ModelInfoError> {
        Ok(self.entries.get(model).cloned())
    }
}

struct Unavailable;

impl ModelInfoSource for Unavailable {
    fn get_model_info(&self, _model: &str) -> Result<Option<JsonObject>, ModelInfoError> {
        Err(ModelInfoError::Unavailable)
    }
}

fn info(json: &str) -> JsonObject {
    serde_json::from_str(json).unwrap()
}

fn amount(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn custom(model: &str, input: Option<&str>, output: Option<&str>) -> Deployment {
    Deployment {
        litellm_params: DeploymentParams {
            model: model.to_owned(),
            input_cost_per_token: input.map(amount),
            output_cost_per_token: output.map(amount),
        },
    }
}

fn catalog(model: &str, json: &str) -> Catalog {
    let mut entries = BTreeMap::new();
    entries.insert(model.to_owned(), info(json));
    Catalog { entries }
}

const TIERED: &str = concat!(
    "{\"input_cost_per_token\":0.25,\"output_cost_per_token\":0.5,",
    "\"input_cost_per_token_above_200k_tokens\":1.0,",
    "\"output_cost_per_token_above_200k_tokens\":2.0,",
    "\"input_cost_per_token_above_128k_tokens\":1.5,",
    "\"output_cost_per_token_above_128k_tokens\":2.5}"
);

#[test]
fn a_deployments_own_rates_price_the_estimate() {
    let deployments = [custom("lens-model", Some("0.25"), Some("0.5"))];
    let estimate = quote(&deployments, "hello", &catalog("lens-model", TIERED)).unwrap();
    assert_eq!(estimate, amount("4791.5"));
}

#[test]
fn the_highest_tier_rate_of_any_deployment_wins() {
    let deployments = [
        custom("first", Some("0.25"), Some("0.5")),
        custom("second", Some("0.5"), Some("1.0")),
    ];
    let estimate = quote(&deployments, "hello", &Catalog::default()).unwrap();
    assert_eq!(estimate, amount("9583.0"));
}

#[test]
fn a_catalog_model_uses_its_highest_tier_rate() {
    let deployments = [custom("lens-model", None, None)];
    let estimate = quote(&deployments, "hello", &catalog("lens-model", TIERED)).unwrap();
    assert_eq!(estimate, amount("24653.0"));
}

#[test]
fn the_prompt_is_measured_in_utf8_bytes() {
    let deployments = [custom("lens-model", Some("0.25"), Some("0.5"))];
    let estimate = quote(&deployments, "日本", &Catalog::default()).unwrap();
    assert_eq!(estimate, amount("4792.0"));
}

#[test]
fn one_custom_rate_falls_back_to_the_catalog() {
    let deployments = [custom("lens-model", Some("0.5"), None)];
    let catalog = catalog(
        "lens-model",
        "{\"input_cost_per_token\":0.25,\"output_cost_per_token\":0.5}",
    );
    let estimate = quote(&deployments, "hello", &catalog).unwrap();
    assert_eq!(estimate, amount("4791.5"));
}

#[test]
fn a_catalog_total_without_tier_rates_prices_the_base_rate() {
    let deployments = [custom("lens-model", None, None)];
    let catalog = catalog(
        "lens-model",
        "{\"input_cost_per_token\":0.25,\"output_cost_per_token\":0.5}",
    );
    let estimate = quote(&deployments, "hello", &catalog).unwrap();
    assert_eq!(estimate, amount("4791.5"));
}

#[test]
fn an_unknown_catalog_model_is_an_error() {
    let deployments = [custom("lens-model", None, None)];
    let error = quote(&deployments, "hello", &Catalog::default()).unwrap_err();
    assert!(matches!(error, QuoteError::UnknownModel { model } if model == "lens-model"));
}

#[test]
fn an_unavailable_catalog_is_an_error() {
    let deployments = [custom("lens-model", None, None)];
    let error = quote(&deployments, "hello", &Unavailable).unwrap_err();
    assert!(matches!(error, QuoteError::Prices { model, .. } if model == "lens-model"));
}

#[test]
fn a_catalog_model_without_a_base_rate_is_an_error() {
    let deployments = [custom("lens-model", None, None)];
    let catalog = catalog("lens-model", "{\"output_cost_per_token\":0.5}");
    let error = quote(&deployments, "hello", &catalog).unwrap_err();
    assert!(matches!(
        error,
        QuoteError::MissingRate { rate, .. } if rate == "input_cost_per_token"
    ));
}

#[test]
fn a_negative_catalog_rate_is_an_error() {
    let deployments = [custom("lens-model", None, None)];
    let catalog = catalog(
        "lens-model",
        "{\"input_cost_per_token\":-0.5,\"output_cost_per_token\":0.5}",
    );
    let error = quote(&deployments, "hello", &catalog).unwrap_err();
    assert!(matches!(
        error,
        QuoteError::NegativeRate { rate, .. } if rate == "input_cost_per_token"
    ));
}

#[test]
fn a_negative_custom_rate_is_an_error() {
    let deployments = [custom("lens-model", Some("0.25"), Some("-0.5"))];
    let error = quote(&deployments, "hello", &Catalog::default()).unwrap_err();
    assert!(matches!(
        error,
        QuoteError::NegativeRate { rate, .. } if rate == "output_cost_per_token"
    ));
}

#[test]
fn a_text_rate_is_an_error() {
    let deployments = [custom("lens-model", None, None)];
    let catalog = catalog(
        "lens-model",
        "{\"input_cost_per_token\":\"0.25\",\"output_cost_per_token\":0.5}",
    );
    let error = quote(&deployments, "hello", &catalog).unwrap_err();
    assert!(matches!(error, QuoteError::InvalidRate { .. }));
}

#[test]
fn a_null_tier_rate_prices_at_zero() {
    let deployments = [custom("lens-model", None, None)];
    let catalog = catalog(
        "lens-model",
        concat!(
            "{\"input_cost_per_token\":0.25,\"output_cost_per_token\":0.5,",
            "\"input_cost_per_token_above_200k_tokens\":null,",
            "\"output_cost_per_token_above_200k_tokens\":null}"
        ),
    );
    let estimate = quote(&deployments, "hello", &catalog).unwrap();
    assert_eq!(estimate, amount("4791.5"));
}

#[test]
fn no_deployments_cannot_be_priced() {
    let error = quote(&[], "hello", &Catalog::default()).unwrap_err();
    assert!(matches!(error, QuoteError::NoDeployments));
}
