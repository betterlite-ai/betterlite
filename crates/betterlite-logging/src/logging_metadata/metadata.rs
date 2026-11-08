use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::fields::payload::records::StandardLoggingMetadata;
use crate::logging_metadata::decode::{DecodeError, decode};
use crate::logging_metadata::hash::is_valid_sha256_hash;
use crate::logging_metadata::keys::STANDARD_METADATA_KEYS;
use crate::logging_metadata::params::MetadataMap;

// the module-level twin also copies used_client_oauth_token, unlike the setup method
pub fn get_standard_logging_metadata(
    metadata: Option<&MetadataMap>,
) -> Result<StandardLoggingMetadata, DecodeError> {
    let Some(fields) = metadata else {
        return Ok(StandardLoggingMetadata::default());
    };
    let mut record: StandardLoggingMetadata =
        decode("standard logging metadata", copied_fields(fields))?;
    if let Some(Json::Text(candidate)) = fields.get("user_api_key")
        && is_valid_sha256_hash(candidate)
    {
        record.user_api_key_hash = Some(candidate.clone());
    }
    Ok(record)
}

fn copied_fields(metadata: &MetadataMap) -> IndexMap<String, Json> {
    let mut fields = IndexMap::new();
    for key in STANDARD_METADATA_KEYS {
        if let Some(value) = metadata.get(key) {
            fields.insert(key.to_owned(), value.clone());
        }
    }
    fields
}

mod tests;
