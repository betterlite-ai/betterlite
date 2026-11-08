use betterlite_models::decisions::Json;

use crate::fields::payload::records::StandardLoggingMetadata;
use crate::logging_metadata::decode::{DecodeError, decode};
use crate::logging_metadata::hash::is_valid_sha256_hash;
use crate::logging_metadata::keys::STANDARD_METADATA_KEYS;
use crate::logging_metadata::params::MetadataMap;

pub fn get_standard_logging_metadata(
    metadata: Option<&MetadataMap>,
) -> Result<StandardLoggingMetadata, DecodeError> {
    let Some(fields) = metadata else {
        return Ok(StandardLoggingMetadata::default());
    };
    let copied = STANDARD_METADATA_KEYS
        .iter()
        .filter_map(|key| Some(((*key).to_owned(), fields.get(*key)?.clone())))
        .collect();
    let mut record: StandardLoggingMetadata = decode("standard logging metadata", copied)?;
    if let Some(Json::Text(candidate)) = fields.get("user_api_key")
        && is_valid_sha256_hash(candidate)
    {
        record.user_api_key_hash = Some(candidate.clone());
    }
    Ok(record)
}

mod tests;
