use secrecy::SecretString;

use super::SecretManagerClient;
use super::error::SecretDispatchError;
use super::settings::SecretManagerSettings;

pub(super) fn read(
    client: &SecretManagerClient,
    secret_name: &str,
    settings: Option<&SecretManagerSettings>,
) -> Result<Option<SecretString>, SecretDispatchError> {
    let SecretManagerClient::AwsSecretManager(handle) = client else {
        return Ok(None);
    };
    let primary_secret_name = settings.and_then(|provided| provided.primary_secret_name.as_deref());
    handle
        .sync_read_secret(secret_name, primary_secret_name)
        .map_err(SecretDispatchError::AwsSecretManager)
}
