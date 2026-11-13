#![cfg(test)]

use betterlite_models::decisions::ir::{DecisionResponse, DecisionUsage};
use betterlite_models::decisions::{Extra, Json, TokenCount};
use rust_decimal::Decimal;

use super::OpenRouterDecisionsConfig;
use crate::decisions::transformation::provider_config::DecisionsProviderConfig;

fn response(extra: Extra) -> DecisionResponse {
    DecisionResponse {
        model: None,
        answers: Vec::new(),
        usage: DecisionUsage {
            input_tokens: TokenCount::default(),
            output_tokens: TokenCount::default(),
            cached_tokens: TokenCount::default(),
            cache_write_tokens: TokenCount::default(),
            reasoning_tokens: TokenCount::default(),
            extra,
        },
        extra: Extra::new(),
    }
}

fn with_cost(cost: Json) -> DecisionResponse {
    let mut extra = Extra::new();
    extra.insert(String::from("cost"), cost);
    response(extra)
}

#[test]
fn the_config_names_the_openrouter_decisions_endpoint() {
    let config = OpenRouterDecisionsConfig;
    assert_eq!(OpenRouterDecisionsConfig::PATH, "/alpha/decisions");
    assert_eq!(OpenRouterDecisionsConfig::API_KEY_ENV.len(), 1);
    assert_eq!(
        OpenRouterDecisionsConfig::API_KEY_ENV[0],
        "OPENROUTER_API_KEY"
    );
    assert_eq!(OpenRouterDecisionsConfig::API_BASE_ENV.len(), 1);
    assert_eq!(
        OpenRouterDecisionsConfig::API_BASE_ENV[0],
        "OPENROUTER_API_BASE"
    );
    assert_eq!(
        config.get_default_api_base(),
        Some(String::from("https://openrouter.ai/api"))
    );
}

#[test]
fn the_complete_url_joins_the_path_after_one_v1() {
    let config = OpenRouterDecisionsConfig;
    assert_eq!(
        config.get_complete_url("https://openrouter.ai/api/v1", "m"),
        "https://openrouter.ai/api/alpha/decisions"
    );
    assert_eq!(
        config.get_complete_url("https://proxy.example/custom", "m"),
        "https://proxy.example/custom/alpha/decisions"
    );
}

#[test]
fn provider_reported_cost_reads_a_decimal_number() {
    let cost = Json::Number(serde_json::Number::from_f64(0.0123).unwrap());
    assert_eq!(
        OpenRouterDecisionsConfig.provider_reported_cost(&with_cost(cost)),
        Some(Decimal::new(123, 4))
    );
}

#[test]
fn provider_reported_cost_reads_an_integer() {
    let cost = Json::Number(serde_json::Number::from(2));
    assert_eq!(
        OpenRouterDecisionsConfig.provider_reported_cost(&with_cost(cost)),
        Some(Decimal::from(2))
    );
}

#[rstest::rstest]
#[case::null(Json::Null)]
#[case::text(Json::Text(String::from("0.5")))]
#[case::boolean(Json::Bool(true))]
#[case::list(Json::List(Vec::new()))]
#[case::object(Json::Object(Extra::new()))]
fn provider_reported_cost_ignores_a_non_number(#[case] cost: Json) {
    assert_eq!(
        OpenRouterDecisionsConfig.provider_reported_cost(&with_cost(cost)),
        None
    );
}

#[test]
fn provider_reported_cost_ignores_a_missing_key() {
    assert_eq!(
        OpenRouterDecisionsConfig.provider_reported_cost(&response(Extra::new())),
        None
    );
}
