use betterlite_config::sections::key_access::Settings;
use secrecy::SecretString;

use super::error::KeyAccessError;

// _check_custom_key_allowed: any supplied value, "" included, is a custom key; only the warning names the switch
pub fn check_custom_key_allowed(
    custom_key: Option<&SecretString>,
    settings: &Settings,
) -> Result<(), KeyAccessError> {
    if custom_key.is_none() || !settings.disable_custom_api_keys {
        return Ok(());
    }
    tracing::warn!("Custom API key rejected: disable_custom_api_keys is enabled");
    Err(KeyAccessError::CustomKeyDisabled)
}
