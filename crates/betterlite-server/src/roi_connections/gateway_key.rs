use betterlite_roi::settings::ROISettings;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use super::error::RoiConnectionsError;

pub const MISSING_KEY: &str = "Add an estimator API key in Advanced settings.";

pub fn gateway_key(
    settings: &ROISettings,
    master_key: Option<&SecretString>,
) -> Result<SecretString, RoiConnectionsError> {
    let estimator = settings.estimator_key.expose_secret();
    if !estimator.is_empty() {
        return Ok(settings.estimator_key.clone());
    }
    let Some(credential) = master_key else {
        return Err(RoiConnectionsError::Conflict(MISSING_KEY));
    };
    if credential.expose_secret().is_empty() {
        return Err(RoiConnectionsError::Conflict(MISSING_KEY));
    }
    Ok(credential.clone())
}

mod tests;
