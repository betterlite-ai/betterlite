use std::{collections::BTreeMap, str};

use super::{AwsKmsError, DecryptedValue, decode_ciphertext};
use aws_sdk_kms::{Client, primitives::Blob};
use secrecy::SecretString;

pub async fn decrypt_value(
    client: &Client,
    environment: &BTreeMap<String, String>,
    secret_name: &str,
) -> Result<DecryptedValue, AwsKmsError> {
    let encrypted_value = environment
        .get(secret_name)
        .ok_or_else(|| AwsKmsError::MissingValue(secret_name.to_owned()))?;
    decrypt_with(client, encrypted_value).await
}

pub(crate) async fn decrypt_with(
    client: &Client,
    encrypted_value: &str,
) -> Result<DecryptedValue, AwsKmsError> {
    let ciphertext = decode_ciphertext(encrypted_value)?;
    let response = client
        .decrypt()
        .ciphertext_blob(Blob::new(ciphertext))
        .send()
        .await
        .map_err(|source| AwsKmsError::Request(Box::new(source)))?;
    let bytes = response
        .plaintext()
        .ok_or(AwsKmsError::MissingPlaintext)?
        .as_ref();
    if bytes.len() > 4096 {
        return Err(AwsKmsError::ValueTooLarge);
    }
    let plaintext = str::from_utf8(bytes)?;
    Ok(parse_plaintext(plaintext))
}

fn parse_plaintext(plaintext: &str) -> DecryptedValue {
    let value = plaintext.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    });
    match value {
        "True" => DecryptedValue::Boolean(true),
        "False" => DecryptedValue::Boolean(false),
        _ => DecryptedValue::String(SecretString::from(value.to_owned())),
    }
}

#[path = "values/boolean_parsing/tests.rs"]
mod boolean_parsing;
#[path = "values/prefix_stripping/tests.rs"]
mod prefix_stripping;
