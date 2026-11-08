use std::collections::BTreeMap;
use std::str::from_utf8;

use secrecy::SecretString;

use super::SecretManagerClient;
use super::base64_codec;
use super::error::SecretDispatchError;

pub(super) fn read(
    client: &SecretManagerClient,
    secret_name: &str,
    environment: &BTreeMap<String, String>,
) -> Result<Option<SecretString>, SecretDispatchError> {
    let Some(encrypted_value) = environment.get(secret_name) else {
        return Ok(None);
    };
    let ciphertext_blob = base64_codec::decode_lenient_ciphertext(encrypted_value)
        .map_err(SecretDispatchError::AwsKmsBase64)?;
    let SecretManagerClient::AwsKms(handle) = client else {
        return Err(SecretDispatchError::WrongClient {
            manager: "AWS KMS",
            expected: "AWSKeyManagementService",
            actual: client.class_name(),
        });
    };
    let plaintext = handle
        .decrypt(&ciphertext_blob)
        .map_err(SecretDispatchError::AwsKms)?;
    let text = from_utf8(&plaintext).map_err(SecretDispatchError::AwsKmsUtf8)?;
    Ok(Some(SecretString::from(text.trim())))
}
