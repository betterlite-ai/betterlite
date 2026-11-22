use betterlite_db::identity_rows::Payload;
use betterlite_logging::callback_config::FeatureFlags;
use betterlite_logging::callback_config::parse::validate_logging_metadata;

use super::error::KeyAccessError;

// raise_on_invalid_key_logging_config: key metadata gets the same logging rules the team callback route applies
pub fn check_key_logging_config(
    metadata: Option<&Payload>,
    flags: FeatureFlags,
) -> Result<(), KeyAccessError> {
    let Some(fields) = metadata else {
        return Ok(());
    };
    let bytes = serde_json::to_vec(fields).map_err(KeyAccessError::Metadata)?;
    validate_logging_metadata(&bytes, flags)
        .map_err(|error| KeyAccessError::LoggingMetadata(Box::new(error)))
}
