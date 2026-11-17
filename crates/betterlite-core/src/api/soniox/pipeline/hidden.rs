use betterlite_config::sections::settings::JsonValue;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use super::error::{DURATION_FIELD, PipelineError};
use crate::api::soniox::records::response::SonioxTranscriptionResponse;

pub const MODEL_FIELD: &str = "model";
pub const PROVIDER_FIELD: &str = "custom_llm_provider";
pub const DURATION_HIDDEN_FIELD: &str = "audio_transcription_duration";
pub const PROVIDER: &str = "soniox";
const MILLIS_PER_SECOND: u32 = 1000;

// LiteLLM sets the three fields in one dict literal, so a bad duration leaves every field unset
pub(super) fn apply(
    response: &mut SonioxTranscriptionResponse,
    model: &str,
    audio_duration_ms: Option<&JsonValue>,
) -> Result<(), PipelineError> {
    let converted = match audio_duration_ms {
        Some(value) => seconds(value)?,
        None => JsonValue::Null,
    };
    response
        .hidden_params
        .insert(MODEL_FIELD.to_owned(), JsonValue::String(model.to_owned()));
    response.hidden_params.insert(
        PROVIDER_FIELD.to_owned(),
        JsonValue::String(PROVIDER.to_owned()),
    );
    response
        .hidden_params
        .insert(DURATION_HIDDEN_FIELD.to_owned(), converted);
    Ok(())
}

fn seconds(value: &JsonValue) -> Result<JsonValue, PipelineError> {
    let millis = match value {
        JsonValue::Number(number) => decimal_millis(&number.to_string()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::String(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    };
    millis
        .and_then(|quantity| quantity.checked_div(Decimal::from(MILLIS_PER_SECOND)))
        .and_then(|exact| exact.to_f64())
        .and_then(Number::from_f64)
        .map(JsonValue::Number)
        .ok_or_else(|| PipelineError::Duration {
            field: DURATION_FIELD,
            value: value.to_string(),
        })
}

fn decimal_millis(text: &str) -> Option<Decimal> {
    Decimal::from_str_exact(text)
        .or_else(|_ignored| Decimal::from_scientific(text))
        .ok()
}
