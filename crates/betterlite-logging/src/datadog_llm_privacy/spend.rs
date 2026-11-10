use betterlite_models::decisions::Json;

use super::datadog_budget_reset_decoder::decode_budget_reset;
use super::json_number::number as json_number;
use crate::datadog_llm_obs::span_records::DDLLMObsSpendMetrics;
use crate::datadog_llm_obs::values::Object;
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::timestamps::with_fraction;

#[must_use]
pub fn spend_metrics(payload: &StandardLoggingPayload) -> DDLLMObsSpendMetrics {
    let metadata = &payload.metadata;
    DDLLMObsSpendMetrics {
        response_cost: Some(payload.response_cost),
        user_api_key_spend: metadata
            .user_api_key_spend
            .map(|value| with_fraction(value.0)),
        user_api_key_max_budget: metadata
            .user_api_key_max_budget
            .map(|value| with_fraction(value.0)),
        user_api_key_budget_reset_at: metadata
            .user_api_key_budget_reset_at
            .as_deref()
            .and_then(decoded_reset),
    }
}

#[must_use]
pub fn metrics_object(metrics: &DDLLMObsSpendMetrics) -> Object {
    let mut object = Object::new();
    if let Some(cost) = metrics.response_cost {
        object.insert("response_cost".to_owned(), json_number(cost));
    }
    if let Some(spend) = metrics.user_api_key_spend {
        object.insert("user_api_key_spend".to_owned(), json_number(spend));
    }
    if let Some(budget) = metrics.user_api_key_max_budget {
        object.insert("user_api_key_max_budget".to_owned(), json_number(budget));
    }
    if let Some(reset_at) = &metrics.user_api_key_budget_reset_at {
        object.insert(
            "user_api_key_budget_reset_at".to_owned(),
            Json::Text(reset_at.clone()),
        );
    }
    object
}

fn decoded_reset(text: &str) -> Option<String> {
    match decode_budget_reset(text) {
        Ok(reset_at) => Some(reset_at),
        Err(error) => {
            tracing::debug!(
                ?error,
                value = text,
                "the budget reset is not an ISO 8601 datetime"
            );
            None
        }
    }
}

mod tests;
