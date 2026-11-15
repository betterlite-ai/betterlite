use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::call_helpers::metadata::MetadataSlots;
use betterlite_prompts::message_content::Message;

use crate::guardrail_records::record::GuardrailStatus;
use crate::model_armor::decisions::append_armor_response;
use crate::model_armor::request_files::{FileScanSettings, scan_request_files};
use crate::model_armor::transport::ModelArmorTransport;
use crate::model_armor::transport::tokens::ArmorTokens;

use super::Data;
use super::error_handling::{FieldError, HookError};

const METADATA_KEY: &str = "metadata";
const LITELLM_METADATA_KEY: &str = "litellm_metadata";
const STATUS_KEY: &str = "_model_armor_status";
const RESPONSE_KEY: &str = "_model_armor_response";
const START_TIME_KEY: &str = "start_time";
// serde_json's arbitrary_precision hands a parsed float to GuardrailValue as this one-key object
const NUMBER_KEY: &str = "$serde_json::private::Number";
pub(super) const BLOCKED: &str = "blocked";
pub(super) const SUCCESS: &str = "success";

// LiteLLM's get_metadata_variable_name_from_kwargs: the newer key wins whenever the request carries it
pub(super) fn bucket_key(data: &Data) -> &'static str {
    if data.contains_key(LITELLM_METADATA_KEY) {
        LITELLM_METADATA_KEY
    } else {
        METADATA_KEY
    }
}

fn bucket(data: &Data) -> Option<&BTreeMap<String, GuardrailValue>> {
    match data.get(bucket_key(data)) {
        Some(GuardrailValue::Object(fields)) => Some(fields),
        _ => None,
    }
}

// LiteLLM's `metadata.get("_model_armor_response", {})`
pub(super) fn recorded_response(data: &Data) -> GuardrailValue {
    bucket(data)
        .and_then(|fields| fields.get(RESPONSE_KEY).cloned())
        .unwrap_or_else(|| GuardrailValue::Object(BTreeMap::new()))
}

// LiteLLM stores "blocked" in a field its GuardrailStatus type does not declare; it maps to intervened here
pub(super) fn recorded_status(data: &Data) -> GuardrailStatus {
    let Some(GuardrailValue::Text(status)) = bucket(data).and_then(|fields| fields.get(STATUS_KEY))
    else {
        return GuardrailStatus::Success;
    };
    match status.as_str() {
        BLOCKED => GuardrailStatus::GuardrailIntervened,
        SUCCESS => GuardrailStatus::Success,
        other => {
            tracing::warn!(
                status = other,
                "Model Armor: unknown status in the metadata bucket"
            );
            GuardrailStatus::Success
        }
    }
}

// LiteLLM's metadata writes after a text scan: append the payload and keep a blocked status sticky
pub(super) fn record_text_scan(data: &mut Data, response: GuardrailValue, blocked: bool) {
    let key = bucket_key(data);
    let mut bucket = match data.remove(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        _ => BTreeMap::new(),
    };
    let existing = bucket.get(RESPONSE_KEY).cloned();
    bucket.insert(
        RESPONSE_KEY.to_owned(),
        append_armor_response(existing, response),
    );
    let sticky = matches!(
        bucket.get(STATUS_KEY),
        Some(GuardrailValue::Text(status)) if status == BLOCKED
    );
    let status = if blocked || sticky { BLOCKED } else { SUCCESS };
    bucket.insert(
        STATUS_KEY.to_owned(),
        GuardrailValue::Text(status.to_owned()),
    );
    data.insert(key.to_owned(), GuardrailValue::Object(bucket));
}

// LiteLLM reads the request's start_time as it is; the record field is a float, so a number is the whole input
pub(super) fn request_start_time(data: &Data) -> Result<Option<f64>, FieldError> {
    match data.get(START_TIME_KEY) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(value) => number_of(value).map(Some).ok_or(FieldError {
            field: START_TIME_KEY,
            expected: "a number",
        }),
    }
}

fn number_of(value: &GuardrailValue) -> Option<f64> {
    match value {
        GuardrailValue::Decimal(number) => Some(*number),
        GuardrailValue::SignedInteger(number) => serde_json::Number::from(*number).as_f64(),
        GuardrailValue::UnsignedInteger(number) => serde_json::Number::from(*number).as_f64(),
        GuardrailValue::Object(fields) => match fields.get(NUMBER_KEY) {
            Some(GuardrailValue::Text(text)) if fields.len() == 1 => text.parse().ok(),
            _ => None,
        },
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::Text(_)
        | GuardrailValue::Array(_) => None,
    }
}

// LiteLLM scans files into the same per-request bucket the text scan later appends to
pub(super) async fn scan_files<T: ArmorTokens>(
    data: &mut Data,
    messages: &[Message],
    transport: &ModelArmorTransport<T>,
    settings: &FileScanSettings,
) -> Result<(), HookError<T::Error>> {
    let key = bucket_key(data);
    let previous = data.get(key).cloned();
    let mut slots = MetadataSlots::default();
    if let Some(GuardrailValue::Object(fields)) = &previous {
        slots.metadata = Some(serde_json::to_value(fields).and_then(serde_json::from_value)?);
    }
    let scanned = scan_request_files(messages, &mut slots, transport, settings).await;
    if let Some(scanned_bucket) = slots.metadata.take() {
        let merged: GuardrailValue = serde_json::from_value(serde_json::to_value(scanned_bucket)?)?;
        if previous.as_ref() != Some(&merged) {
            data.insert(key.to_owned(), merged);
        }
    }
    scanned.map_err(HookError::from)
}

mod tests;
