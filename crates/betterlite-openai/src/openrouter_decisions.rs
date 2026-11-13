mod tests;

use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_models::decisions::ir::DecisionResponse;
use rust_decimal::Decimal;

use crate::decisions::transformation::provider_config::DecisionsProviderConfig;

pub struct OpenRouterDecisionsConfig;

impl DecisionsProviderConfig for OpenRouterDecisionsConfig {
    const PATH: &'static str = "/alpha/decisions";
    const API_KEY_ENV: &'static [&'static str] = &["OPENROUTER_API_KEY"];
    const API_BASE_ENV: &'static [&'static str] = &["OPENROUTER_API_BASE"];

    fn get_default_api_base(&self) -> Option<String> {
        Some(String::from("https://openrouter.ai/api"))
    }

    fn provider_reported_cost(&self, response: &DecisionResponse) -> Option<Decimal> {
        let Json::Number(number) = response.usage.extra.get("cost")? else {
            return None;
        };
        let text = number.to_string();
        Decimal::from_str(&text)
            .or_else(|_| Decimal::from_scientific(&text))
            .ok()
    }
}
