mod tests;

use std::convert::Infallible;

use betterlite_db::stored_cipher::DbCipher;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::gateway_dcr::contracts::{CLIENT_RECORD_DEBUG_KEY, GATEWAY_DCR_CLIENT_ID_PREFIX};
use crate::gateway_dcr::models::GatewayDcrClient;

#[derive(Debug, Error)]
pub enum SealError {
    #[error("the value to seal could not be serialized")]
    Serialize(#[source] serde_json::Error),
    #[error("the value to seal could not be encrypted")]
    Encrypt(#[source] SecretError),
}

pub fn seal<P>(prefix: &str, payload: &P, cipher: &DbCipher) -> Result<String, SealError>
where
    P: Serialize,
{
    let serialized = serde_json::to_string(payload).map_err(SealError::Serialize)?;
    let stored = StoredValue::<Infallible>::Text(SecretString::from(serialized));
    let sealed = cipher
        .encrypt_value(stored, None)
        .map_err(SealError::Encrypt)?;
    let text = match sealed {
        StoredValue::Text(text) => text,
        StoredValue::Other(never) => match never {},
    };
    Ok(format!("{prefix}{}", text.expose_secret()))
}

pub trait Sealable: DeserializeOwned {
    #[must_use]
    fn acceptable(&self) -> bool;
}

#[must_use]
pub fn open_sealed<T>(value: &str, prefix: &str, cipher: &DbCipher, debug_key: &str) -> Option<T>
where
    T: Sealable,
{
    let ciphertext = value.strip_prefix(prefix)?;
    let stored = StoredValue::<Infallible>::Text(SecretString::from(ciphertext.to_owned()));
    let opened = cipher.decrypt_value(
        stored,
        debug_key,
        DecryptPolicy {
            log: FailureLog::Error,
            value: FailureValue::Nothing,
        },
    )?;
    let text = match opened {
        StoredValue::Text(text) => text,
        StoredValue::Other(never) => match never {},
    };
    let parsed: T = serde_json::from_str(text.expose_secret()).ok()?;
    parsed.acceptable().then_some(parsed)
}

#[must_use]
pub fn open_gateway_dcr_client(client_id: &str, cipher: &DbCipher) -> Option<GatewayDcrClient> {
    open_sealed(
        client_id,
        GATEWAY_DCR_CLIENT_ID_PREFIX,
        cipher,
        CLIENT_RECORD_DEBUG_KEY,
    )
}
