use super::datadog_budget_reset_decoder::decode_budget_reset;
use crate::datadog_llm_obs::span_records::DDLLMObsSpendMetrics;
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
