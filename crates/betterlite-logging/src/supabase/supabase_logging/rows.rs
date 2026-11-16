use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Serialize;

#[derive(Serialize)]
pub struct InitiatedRow {
    pub model: String,
    pub messages: Json,
    pub end_user: Json,
    pub status: &'static str,
    pub litellm_call_id: String,
}

pub struct RequestRow<'row> {
    pub messages: Option<&'row Json>,
    pub end_user: Option<&'row str>,
    pub response_obj: &'row Json,
    pub start_time: Decimal,
    pub end_time: Decimal,
    pub litellm_call_id: &'row str,
    pub total_cost: Decimal,
}

// LiteLLM's log_event builds one dictionary per branch, so the keys it does not set are absent
#[derive(Serialize)]
pub struct TerminalRow<'row> {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub response_time: Decimal,
    pub model: &'row Json,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub total_cost: Decimal,
    pub messages: Option<&'row Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<&'row Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'row Json>,
    pub end_user: Option<&'row str>,
    pub litellm_call_id: &'row str,
    pub status: &'row str,
}
