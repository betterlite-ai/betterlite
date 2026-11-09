use secrecy::SecretString;

use super::SecretManagerClient;
use super::error::SecretDispatchError;

pub(super) fn read(
    client: &SecretManagerClient,
    secret_name: &str,
) -> Result<Option<SecretString>, SecretDispatchError> {
    let SecretManagerClient::AzureKeyVault(handle) = client else {
        return Err(SecretDispatchError::WrongClient {
            manager: "Azure Key Vault",
            expected: "SecretClient",
            actual: client.class_name(),
        });
    };
    let secret = handle
        .get_secret(secret_name)
        .map_err(SecretDispatchError::AzureKeyVault)?;
    Ok(secret.value)
}
