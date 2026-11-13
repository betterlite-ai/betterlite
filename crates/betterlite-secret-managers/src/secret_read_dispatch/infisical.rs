use secrecy::SecretString;

use super::SecretManagerClient;
use super::error::SecretDispatchError;

pub(super) fn read(
    client: &SecretManagerClient,
    secret_name: &str,
) -> Result<Option<SecretString>, SecretDispatchError> {
    let SecretManagerClient::Infisical(handle) = client else {
        return Err(SecretDispatchError::WrongClient {
            manager: "Infisical",
            expected: "InfisicalClient",
            actual: client.class_name(),
        });
    };
    handle
        .get_secret(secret_name)
        .map(|secret| secret.secret_value)
        .map_err(SecretDispatchError::Infisical)
}
