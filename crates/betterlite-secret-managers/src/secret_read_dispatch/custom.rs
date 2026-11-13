use secrecy::SecretString;

use super::SecretManagerClient;
use super::error::SecretDispatchError;
use super::settings::SecretManagerSettings;

pub(super) fn read(
    client: &SecretManagerClient,
    secret_name: &str,
    settings: Option<&SecretManagerSettings>,
) -> Result<Option<SecretString>, SecretDispatchError> {
    let SecretManagerClient::Custom(handle) = client else {
        return Err(SecretDispatchError::CustomWrongClient {
            actual: client.class_name(),
        });
    };
    let optional_params = settings.map(|provided| &provided.optional_params);
    let secret = handle
        .sync_read_secret(secret_name, optional_params)
        .map_err(SecretDispatchError::Custom)?;
    secret
        .map(Some)
        .ok_or_else(|| SecretDispatchError::CustomMissing {
            secret_name: secret_name.to_owned(),
        })
}
