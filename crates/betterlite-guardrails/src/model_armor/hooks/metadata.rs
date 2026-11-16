use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::call_helpers::metadata::{
    MetadataBucket, MetadataMap, MetadataSlots, metadata_variable_name,
};
use betterlite_prompts::message_content::Message;

use crate::guardrail_records::record::GuardrailStatus;
use crate::model_armor::decisions::append_armor_response;
use crate::model_armor::request_files::{FileScanSettings, scan_request_files};
use crate::model_armor::transport::ModelArmorTransport;
use crate::model_armor::transport::tokens::ArmorTokens;

use super::Data;
use super::error_handling::HookError;

const STATUS_KEY: &str = "_model_armor_status";
const RESPONSE_KEY: &str = "_model_armor_response";
const START_TIME_KEY: &str = "start_time";
// serde_json's arbitrary_precision hands a parsed float to GuardrailValue as this one-key object
const NUMBER_KEY: &str = "$serde_json::private::Number";
pub(super) const BLOCKED: &str = "blocked";
pub(super) const SUCCESS: &str = "success";

// LiteLLM's get_metadata_variable_name_from_kwargs: the newer key wins whenever the request carries it
pub(super) fn bucket_key(data: &Data) -> &'static str {
    let slots = MetadataSlots {
        metadata: None,
        litellm_metadata: data
            .contains_key(MetadataBucket::LitellmMetadata.key())
            .then(MetadataMap::new),
    };
    metadata_variable_name(&slots).key()
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

// LiteLLM copies the request's start_time into the record unchecked; the record field is a float, so a non-number logs as null
pub(super) fn request_start_time(data: &Data) -> Option<f64> {
    let value = data.get(START_TIME_KEY)?;
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
pub(super) async fn scan_files<T>(
    data: &mut Data,
    messages: &[Message],
    transport: &ModelArmorTransport<T>,
    settings: &FileScanSettings,
) -> Result<(), HookError<T::Error>>
where
    T: ArmorTokens,
{
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
