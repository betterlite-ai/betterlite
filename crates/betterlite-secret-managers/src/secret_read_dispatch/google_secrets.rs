use secrecy::SecretString;

use super::SecretManagerClient;
use super::error::SecretDispatchError;

pub(super) fn read(
    client: &SecretManagerClient,
    secret_name: &str,
) -> Result<Option<SecretString>, SecretDispatchError> {
    let SecretManagerClient::GoogleSecretManager(handle) = client else {
        return Err(SecretDispatchError::WrongClient {
            manager: "Google Secret Manager",
            expected: "GoogleSecretManager",
            actual: client.class_name(),
        });
    };
    let secret = handle
        .get_secret_from_google_secret_manager(secret_name)
        .map_err(SecretDispatchError::GoogleSecretManager)?;
    secret
        .map(Some)
        .ok_or_else(|| SecretDispatchError::GoogleSecretManagerMissing {
            secret_name: secret_name.to_owned(),
        })
}
