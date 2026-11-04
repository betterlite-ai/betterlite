use std::sync::Arc;

use aws_sdk_kms::primitives::Blob;

use super::{AWS_KMS_CLIENT, AwsKmsError, DecryptedValue, decode_ciphertext, environment_snapshot};

pub async fn decrypt_value(secret_name: &str) -> Result<DecryptedValue, AwsKmsError> {
    let environment = environment_snapshot();
    let encrypted_value = environment
        .get(secret_name)
        .ok_or_else(|| AwsKmsError::MissingValue(secret_name.to_owned()))?;
    let client = AWS_KMS_CLIENT
        .lock()
        .map_err(|_poisoned| AwsKmsError::ClientState)?
        .as_ref()
        .map(Arc::clone)
        .ok_or(AwsKmsError::MissingClient)?;
    decrypt_with(&client, encrypted_value).await
}

pub(crate) async fn decrypt_with(
    client: &aws_sdk_kms::Client,
    encrypted_value: &str,
) -> Result<DecryptedValue, AwsKmsError> {
    let ciphertext = decode_ciphertext(encrypted_value)?;
    let response = client
        .decrypt()
        .ciphertext_blob(Blob::new(ciphertext))
        .send()
        .await
        .map_err(|error| AwsKmsError::Request(error.to_string()))?;
    let bytes = response
        .plaintext()
        .ok_or_else(|| AwsKmsError::Request("KMS response omitted Plaintext".to_owned()))?
        .as_ref()
        .to_vec();
    let plaintext = String::from_utf8(bytes)?;
    Ok(parse_plaintext(&plaintext))
}

fn parse_plaintext(plaintext: &str) -> DecryptedValue {
    let value = plaintext.trim();
    match value {
        "True" => DecryptedValue::Boolean(true),
        "False" => DecryptedValue::Boolean(false),
        _ => DecryptedValue::String(value.to_owned()),
    }
}

mod tests;
