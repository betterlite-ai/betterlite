use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

use crate::api::managed_resource_ids::is_base64_encoded_unified_id;

use super::RoutingIdType;
use super::legacy::is_strip_whitespace;

const ROUTING_PREFIX: &str = "litellm:";
const MODEL_FIELD: &str = ";model,";
const MODEL_FIELD_NAME: &str = "model,";
const FILE_ID_PREFIX: &str = "file-";
const BATCH_ID_PREFIX: &str = "batch_";

#[must_use]
pub fn encode_file_id_with_model(file_id: &str, model: &str, id_type: RoutingIdType) -> String {
    let encoded = URL_SAFE_NO_PAD.encode(format!("litellm:{file_id};model,{model}"));
    let prefix = if file_id.starts_with(BATCH_ID_PREFIX) {
        BATCH_ID_PREFIX
    } else if file_id.starts_with(FILE_ID_PREFIX) {
        FILE_ID_PREFIX
    } else if id_type == RoutingIdType::Batch {
        BATCH_ID_PREFIX
    } else {
        FILE_ID_PREFIX
    };
    format!("{prefix}{encoded}")
}

#[must_use]
pub fn decode_model_from_file_id(encoded_id: &str) -> Option<String> {
    model_field(&decode_payload(encoded_id)?)
        .map(|model| model.trim_matches(is_strip_whitespace).to_owned())
}

#[must_use]
pub fn get_original_file_id(encoded_id: &str) -> String {
    decode_payload(encoded_id)
        .as_deref()
        .and_then(original_field)
        .unwrap_or(encoded_id)
        .to_owned()
}

#[must_use]
pub fn is_model_embedded_id(encoded_id: &str) -> bool {
    if let Some(decoded) = decode_payload(encoded_id) {
        return model_field(&decoded).is_some();
    }
    false
}

fn decode_payload(encoded_id: &str) -> Option<String> {
    let payload = encoded_id
        .strip_prefix(FILE_ID_PREFIX)
        .or_else(|| encoded_id.strip_prefix(BATCH_ID_PREFIX))
        .unwrap_or(encoded_id);
    let decoded = is_base64_encoded_unified_id(payload, ROUTING_PREFIX)?;
    decoded.contains(MODEL_FIELD).then_some(decoded)
}

fn model_field(decoded: &str) -> Option<&str> {
    decoded.split(';').skip(1).find_map(|field| {
        field
            .strip_prefix(MODEL_FIELD_NAME)
            .filter(|model| !model.is_empty())
    })
}

fn original_field(decoded: &str) -> Option<&str> {
    decoded
        .split(';')
        .zip(decoded.split(';').skip(1))
        .find_map(|(field, next)| {
            let original = field.split_once(ROUTING_PREFIX)?.1;
            (!original.is_empty() && next.starts_with(MODEL_FIELD_NAME)).then_some(original)
        })
}
