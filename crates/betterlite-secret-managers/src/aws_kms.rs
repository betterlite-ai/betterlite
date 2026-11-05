use std::{
    str::Utf8Error,
    sync::{Arc, Mutex},
};

use aws_sdk_kms::{
    Client, config::http::HttpResponse, error::SdkError, operation::decrypt::DecryptError,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use secrecy::SecretString;
use thiserror::Error;

pub mod environment;
pub mod initialization;
pub mod values;

pub static AWS_KMS_CLIENT: Mutex<Option<Arc<Client>>> = Mutex::new(None);

#[derive(Debug, Error)]
pub enum AwsKmsError {
    #[error("Missing required environment variable - AWS_REGION_NAME")]
    MissingRegion,
    #[error("AWS KMS encrypted value for {0} is missing")]
    MissingValue(String),
    #[error("invalid AWS KMS base64 ciphertext: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("AWS KMS request failed")]
    Request(#[source] Box<SdkError<DecryptError, HttpResponse>>),
    #[error("AWS KMS response omitted Plaintext")]
    MissingPlaintext,
    #[error("AWS KMS value exceeds the size limit")]
    ValueTooLarge,
    #[error("AWS KMS client state is poisoned")]
    ClientState,
    #[error("AWS KMS plaintext is not UTF-8: {0}")]
    Utf8(#[from] Utf8Error),
}

pub enum DecryptedValue {
    Boolean(bool),
    String(SecretString),
}

pub(crate) fn decode_ciphertext(value: &str) -> Result<Vec<u8>, AwsKmsError> {
    if value.len() > 0x0001_0000 {
        return Err(AwsKmsError::ValueTooLarge);
    }
    let encoded = value.strip_prefix("aws_kms/").unwrap_or(value);
    let filtered: Vec<u8> = encoded
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        .collect();
    if filtered.len() > 8192 {
        return Err(AwsKmsError::ValueTooLarge);
    }
    Ok(STANDARD.decode(filtered)?)
}

mod tests;
