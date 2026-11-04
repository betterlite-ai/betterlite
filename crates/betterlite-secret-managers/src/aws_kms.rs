use std::{
    collections::BTreeMap,
    env,
    string::FromUtf8Error,
    sync::{Arc, Mutex},
};

use aws_sdk_kms::Client;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use thiserror::Error;

pub mod environment;
pub mod initialization;
pub mod values;

pub static AWS_KMS_CLIENT: Mutex<Option<Arc<Client>>> = Mutex::new(None);

#[derive(Debug, Error)]
pub enum AwsKmsError {
    #[error("Missing required environment variable - AWS_REGION_NAME")]
    MissingRegion,
    #[error("AWS KMS client has not been initialized")]
    MissingClient,
    #[error("AWS KMS encrypted value for {0} is missing")]
    MissingValue(String),
    #[error("invalid AWS KMS base64 ciphertext: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("AWS KMS request failed: {0}")]
    Request(String),
    #[error("AWS KMS client state is poisoned")]
    ClientState,
    #[error("AWS KMS plaintext is not UTF-8: {0}")]
    Utf8(#[from] FromUtf8Error),
}

#[derive(Debug, Eq, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum DecryptedValue {
    Boolean(bool),
    String(String),
}

pub(crate) fn decode_ciphertext(value: &str) -> Result<Vec<u8>, AwsKmsError> {
    if let Some(encoded) = value.strip_prefix("aws_kms/") {
        return Ok(STANDARD.decode(encoded)?);
    }
    let ciphertext = STANDARD.decode(value)?;
    Ok(ciphertext)
}

#[expect(
    clippy::disallowed_methods,
    reason = "KMS bootstrap explicitly reads startup environment variables"
)]
pub(crate) fn environment_snapshot() -> BTreeMap<String, String> {
    env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}
