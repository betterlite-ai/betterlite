use secrecy::SecretString;

use super::SecretManagerClient;
use super::error::SecretDispatchError;

pub(super) fn read(
    client: &SecretManagerClient,
    secret_name: &str,
) -> Result<Option<SecretString>, SecretDispatchError> {
    let SecretManagerClient::Vault(handle) = client else {
        return Err(SecretDispatchError::WrongClient {
            manager: "Hashicorp Vault",
            expected: "HashicorpSecretManager",
            actual: client.class_name(),
        });
    };
    handle
        .sync_read_secret(secret_name)
        .map(Some)
        .ok_or_else(|| SecretDispatchError::VaultMissing {
            secret_name: secret_name.to_owned(),
        })
}
