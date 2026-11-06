use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use secrecy::SecretString;
use serde::Deserialize;

use super::GoogleSecretManagerError;

const PYTHON_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

#[derive(Deserialize)]
struct AccessResponse {
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    data: Option<String>,
}

pub fn decode_payload(body: &str) -> Result<Option<SecretString>, GoogleSecretManagerError> {
    let response: AccessResponse =
        serde_json::from_str(body).map_err(GoogleSecretManagerError::Response)?;
    let Some(encoded) = response.payload.and_then(|payload| payload.data) else {
        return Ok(None);
    };
    let bytes = PYTHON_BASE64
        .decode(encoded)
        .map_err(GoogleSecretManagerError::Base64)?;
    let text = String::from_utf8(bytes).map_err(GoogleSecretManagerError::Utf8)?;
    Ok(Some(SecretString::from(text)))
}

mod tests;
