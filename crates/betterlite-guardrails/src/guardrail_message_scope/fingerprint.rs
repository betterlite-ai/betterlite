use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::StreamItem;
use error::FingerprintError;

pub mod error;
mod tests;

#[must_use]
pub fn stream_item_field<'item>(
    item: &'item StreamItem,
    field: &str,
) -> Option<&'item GuardrailValue> {
    let StreamItem::Json(GuardrailValue::Object(fields)) = item else {
        return None;
    };
    fields
        .get(field)
        .filter(|value| !matches!(value, GuardrailValue::Null))
}

#[must_use]
pub fn stream_item_items<'item>(item: &'item StreamItem, field: &str) -> &'item [GuardrailValue] {
    match stream_item_field(item, field) {
        Some(GuardrailValue::Array(values)) => values,
        _ => &[],
    }
}

pub fn stream_item_fingerprint(item: &StreamItem) -> Result<String, FingerprintError> {
    match item {
        StreamItem::Json(value) => {
            serde_json::to_string(value).map_err(|source| FingerprintError::Serialize { source })
        }
        StreamItem::Bytes(_) => Err(FingerprintError::Bytes),
    }
}
