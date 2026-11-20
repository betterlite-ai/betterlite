use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::fields::payload::records::StandardLoggingMetadata;
use crate::spend_metadata::as_object;

const REQUESTER_METADATA: &str = "requester_metadata";
const USER_API_KEY_AUTH_METADATA: &str = "user_api_key_auth_metadata";
const SPEND_LOGS_METADATA: &str = "spend_logs_metadata";

#[derive(Debug, thiserror::Error)]
pub enum CombinedMetadataError {
    #[error("the standard logging metadata could not be encoded")]
    Encode(#[source] serde_json::Error),
}

// LiteLLM unpacks the subobjects after the top-level scalars, so a subobject entry overwrites a scalar of the same name
pub fn combined_custom_metadata(
    metadata: Option<&StandardLoggingMetadata>,
) -> Result<JsonObject, CombinedMetadataError> {
    let Some(record) = metadata else {
        return Ok(JsonObject::new());
    };
    let encoded = serde_json::to_value(record)
        .and_then(serde_json::from_value::<Json>)
        .map_err(CombinedMetadataError::Encode)?;
    let Some(fields) = as_object(&encoded) else {
        return Ok(JsonObject::new());
    };
    let mut combined = JsonObject::new();
    for (key, value) in fields {
        if !matches!(value, Json::Null | Json::Object(_)) {
            combined.insert(key.clone(), value.clone());
        }
    }
    for name in [
        REQUESTER_METADATA,
        USER_API_KEY_AUTH_METADATA,
        SPEND_LOGS_METADATA,
    ] {
        if let Some(entries) = fields.get(name).and_then(as_object) {
            for (key, value) in entries {
                combined.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(combined)
}

#[path = "combined_custom_metadata/tests.rs"]
mod tests;
