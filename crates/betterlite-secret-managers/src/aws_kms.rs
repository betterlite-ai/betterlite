use std::{
    str::Utf8Error,
    sync::{Arc, Mutex},
};

use aws_sdk_kms::{
    Client, config::http::HttpResponse, error::SdkError, operation::decrypt::DecryptError,
};
use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use secrecy::SecretString;
use thiserror::Error;

pub mod environment;
pub mod initialization;
pub mod values;

pub static AWS_KMS_CLIENT: Mutex<Option<Arc<Client>>> = Mutex::new(None);

const PYTHON_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone),
);

#[derive(Debug, Error)]
pub enum AwsKmsError {
    #[error("Missing required environment variable - AWS_REGION_NAME")]
    MissingRegion,
    #[error("AWS KMS encrypted value for {0} is missing")]
    MissingValue(String),
    #[error("invalid AWS KMS base64 ciphertext: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("invalid AWS KMS base64 ciphertext: Incorrect padding")]
    Base64Padding,
    #[error(
        "invalid AWS KMS base64 ciphertext: number of data characters ({0}) cannot be 1 more than a multiple of 4"
    )]
    Base64Length(usize),
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
    #[error("AWS KMS secret key pattern is invalid: {0}")]
    KeyPattern(#[from] regex::Error),
}

pub enum DecryptedValue {
    Boolean(bool),
    String(SecretString),
}

pub(crate) fn decode_ciphertext(value: &str) -> Result<Vec<u8>, AwsKmsError> {
    if value.len() > 0x0001_0000 {
        return Err(AwsKmsError::ValueTooLarge);
    }
    let encoded = if value.starts_with("aws_kms/") {
        value.replace("aws_kms/", "")
    } else {
        value.to_owned()
    };
    let mut symbols = Vec::new();
    let mut pads = 0_usize;
    for byte in encoded.bytes() {
        match byte {
            b'=' if symbols.len().rem_euclid(4) >= 2 => pads = pads.saturating_add(1),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => {
                symbols.push(byte);
                pads = 0;
            }
            _ => {}
        }
    }
    if symbols.len() > 8192 {
        return Err(AwsKmsError::ValueTooLarge);
    }
    match symbols.len().rem_euclid(4) {
        1 => Err(AwsKmsError::Base64Length(symbols.len())),
        quad_position @ (2 | 3) if quad_position.saturating_add(pads) < 4 => {
            Err(AwsKmsError::Base64Padding)
        }
        _ => Ok(PYTHON_BASE64.decode(symbols)?),
    }
}

mod tests;
