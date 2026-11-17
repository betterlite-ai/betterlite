use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_with::rust::double_option;

const COMPLETED_STATUS: &str = "COMPLETED";

// LiteLLM's TinyfishRun is a total=False TypedDict: a missing field stays missing and an explicit null stays null.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TinyfishRun {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub run_id: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub status: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub num_of_steps: Option<Option<i64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub result: Option<Option<Json>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub error: Option<Option<Json>>,
    #[serde(
        default,
        rename = "type",
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub run_type: Option<Option<String>>,
}

// LiteLLM validates the payload with a TypeAdapter and warns on a shape it cannot read, keeping the request logged.
#[must_use]
pub fn parse_run(payload: &Json) -> Option<TinyfishRun> {
    match serde_json::to_string(payload).and_then(|text| serde_json::from_str(&text)) {
        Ok(run) => Some(run),
        Err(error) => {
            tracing::warn!(%error, "TinyFish passthrough: unexpected run object shape");
            None
        }
    }
}

// TinyFish invoices COMPLETED runs only, so FAILED, CANCELLED and pending runs charge nothing.
#[must_use]
pub fn run_cost(run: Option<&TinyfishRun>, cost_per_step: Decimal) -> Option<Decimal> {
    let submitted = run?;
    let status = submitted.status.as_ref().and_then(|value| value.as_deref());
    if status != Some(COMPLETED_STATUS) {
        return None;
    }
    let steps = submitted.num_of_steps??;
    Decimal::from(steps).checked_mul(cost_per_step)
}
