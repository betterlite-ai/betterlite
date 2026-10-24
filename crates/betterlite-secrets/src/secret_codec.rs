pub mod bearer;
pub mod diagnostics;
mod python_base64;
mod sealing;
pub mod secret_map;

use std::any::type_name;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use secrecy::{ExposeSecret as _, SecretString};

use crate::error::SecretError;
use crate::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use crate::secret_codec::python_base64::decode_lenient;
use crate::secret_codec::sealing::{open_aes, open_legacy, seal_aes, seal_legacy};

pub const CALLBACK_VAR_ENCRYPTED_PREFIX: &str = "litellm_enc::";
pub const MAX_ENCODED_BYTES: usize = 0x0010_0000;
// a colon never appears in base64url, so the marker tells the two formats apart on read
const AES_PREFIX: &str = "v2:gcm:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    XSalsa20Poly1305,
    Aes256Gcm,
}

#[derive(Debug, Clone)]
pub struct CipherSettings {
    pub signing_key: Option<SecretString>,
    pub algorithm: Algorithm,
}

pub trait NonceSource {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError>;
}

pub struct OsNonces;

impl NonceSource for OsNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        getrandom::fill(nonce).map_err(|source| SecretError::Random { source })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredValue<T> {
    Text(String),
    Other(T),
}

#[derive(Debug, Clone, Copy)]
pub enum FailureLog {
    Debug,
    Error,
}

#[derive(Debug, Clone, Copy)]
pub enum FailureValue {
    Original,
    Nothing,
}

#[derive(Debug, Clone, Copy)]
pub struct DecryptPolicy {
    pub log: FailureLog,
    pub value: FailureValue,
}

pub struct StoredCipher<N, D> {
    pub settings: CipherSettings,
    pub nonces: N,
    pub diagnostics: D,
}

impl<N, D> StoredCipher<N, D>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    pub fn encrypt_value<T>(
        &self,
        value: StoredValue<T>,
        override_key: Option<&SecretString>,
    ) -> Result<StoredValue<T>, SecretError> {
        let StoredValue::Text(text) = value else {
            let event = SecretEvent::NotAString {
                value_type: type_name::<T>(),
            };
            self.diagnostics.record(Level::Debug, &event);
            return Ok(value);
        };
        self.encrypt_text(&text, override_key)
            .map(StoredValue::Text)
    }

    pub(crate) fn encrypt_text(
        &self,
        text: &str,
        override_key: Option<&SecretString>,
    ) -> Result<String, SecretError> {
        let key = override_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(self.settings.signing_key.as_ref())
            .ok_or(SecretError::MissingKey)?;
        Ok(match self.settings.algorithm {
            Algorithm::Aes256Gcm => {
                let sealed = seal_aes(key, &self.nonces, text.as_bytes(), b"")?;
                format!("{AES_PREFIX}{}", URL_SAFE.encode(sealed))
            }
            Algorithm::XSalsa20Poly1305 => {
                URL_SAFE.encode(seal_legacy(key, &self.nonces, text.as_bytes())?)
            }
        })
    }

    pub fn decrypt_value<T>(
        &self,
        value: StoredValue<T>,
        field: &str,
        policy: DecryptPolicy,
    ) -> Option<StoredValue<T>> {
        let StoredValue::Text(text) = value else {
            return Some(value);
        };
        let opened = self
            .settings
            .signing_key
            .as_ref()
            .ok_or(SecretError::MissingKey)
            .and_then(|key| open_stored(&text, key));
        let error = match opened {
            Ok(plaintext) => return Some(StoredValue::Text(plaintext)),
            Err(error) => error,
        };
        let failed = SecretEvent::DecryptFailed {
            field,
            error: &error,
        };
        if matches!(policy.log, FailureLog::Debug) {
            self.diagnostics.record(Level::Debug, &failed);
        } else {
            let unable = SecretEvent::UnableToDecrypt { field };
            self.diagnostics.record(Level::Debug, &unable);
            if matches!(policy.value, FailureValue::Nothing) {
                self.diagnostics.record(Level::Error, &failed);
            }
        }
        match policy.value {
            FailureValue::Original => Some(StoredValue::Text(text)),
            FailureValue::Nothing => None,
        }
    }
}

fn open_stored(text: &str, key: &SecretString) -> Result<String, SecretError> {
    if let Some(encoded) = text.strip_prefix(AES_PREFIX) {
        return open_aes(key, &decode_lenient(encoded, true)?, b"");
    }
    let sealed = decode_lenient(text, true).or_else(|_url| decode_lenient(text, false))?;
    open_legacy(key, &sealed)
}

#[must_use]
pub fn decrypt_if_encrypted_with(value: &str, key: &SecretString) -> Option<String> {
    // "" and "*" decode to no bytes, which the legacy box reads as empty text under any key
    if !value.starts_with(AES_PREFIX)
        && decode_lenient(value, true)
            .or_else(|_url| decode_lenient(value, false))
            .ok()?
            .is_empty()
    {
        return None;
    }
    open_stored(value, key).ok()
}

mod tests;
