use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_secrets::secret_codec::OsNonces;
use betterlite_secrets::stored_cipher::app_crypto::{AppCrypto, AppCryptoError, Envelope};
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};
use serde::Serialize;

use crate::fields::payload::StandardLoggingPayload;

#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("the SQS application-level encryption key is not valid base64")]
    Decode(#[source] base64::DecodeError),
    #[error("the SQS payload could not be encoded as JSON")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Crypto(#[from] AppCryptoError),
}

pub struct Encryption {
    crypto: AppCrypto<OsNonces>,
    aad: Option<String>,
}

// LiteLLM wraps AppCrypto's envelope in this marker so consumers can tell encrypted bodies apart
#[derive(Serialize)]
struct EncryptedMessage {
    #[serde(rename = "__encrypted__")]
    encrypted: bool,
    payload: Envelope,
}

impl Encryption {
    pub fn from_base64(key: &SecretString, aad: Option<String>) -> Result<Self, EncryptionError> {
        let decoded = STANDARD
            .decode(key.expose_secret())
            .map_err(EncryptionError::Decode)?;
        let master = SecretSlice::from(decoded);
        Ok(Self {
            crypto: AppCrypto::new(&master, OsNonces)?,
            aad,
        })
    }

    pub fn wrap(&self, payload: &StandardLoggingPayload) -> Result<String, EncryptionError> {
        let value = serde_json::to_value(payload).map_err(EncryptionError::Encode)?;
        let aad = self.aad.as_deref().unwrap_or_default().as_bytes();
        let envelope = self.crypto.encrypt_json(&value, aad)?;
        serde_json::to_string(&EncryptedMessage {
            encrypted: true,
            payload: envelope,
        })
        .map_err(EncryptionError::Encode)
    }
}
