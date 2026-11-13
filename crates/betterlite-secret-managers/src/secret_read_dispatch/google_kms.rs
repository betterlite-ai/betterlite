use std::collections::BTreeMap;
use std::str::from_utf8;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::SecretString;

use super::SecretManagerClient;
use super::base64_codec;
use super::error::SecretDispatchError;
use super::protocols::GoogleKmsDecryptRequest;

pub(super) fn read(
    client: &SecretManagerClient,
    secret_name: &str,
    environment: &BTreeMap<String, String>,
) -> Result<Option<SecretString>, SecretDispatchError> {
    let Some(encrypted_secret) = environment.get(secret_name) else {
        return Err(SecretDispatchError::GoogleKmsMissingSecret);
    };
    if !base64_codec::is_base64(encrypted_secret) {
        return Err(SecretDispatchError::GoogleKmsInvalidBase64);
    }
    let ciphertext = STANDARD
        .decode(encrypted_secret)
        .map_err(SecretDispatchError::GoogleKmsBase64)?;
    let SecretManagerClient::GoogleKms(handle) = client else {
        return Err(SecretDispatchError::WrongClient {
            manager: "Google KMS",
            expected: "KeyManagementServiceClient",
            actual: client.class_name(),
        });
    };
    let plaintext = handle
        .decrypt(GoogleKmsDecryptRequest {
            name: handle.resource_name(),
            ciphertext: &ciphertext,
        })
        .map_err(SecretDispatchError::GoogleKms)?;
    let text = from_utf8(&plaintext).map_err(SecretDispatchError::GoogleKmsUtf8)?;
    Ok(Some(SecretString::from(text)))
}
