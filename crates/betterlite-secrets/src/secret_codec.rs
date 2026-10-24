pub mod bearer;
pub mod diagnostics;
mod python_base64;
pub mod sealing;
pub mod secret_map;

use std::any::type_name;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use secrecy::{ExposeSecret as _, SecretString};

use crate::error::SecretError;
use crate::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use crate::secret_codec::python_base64::decode_lenient;
use crate::secret_codec::sealing::{open, seal};

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

#[derive(Debug, Clone)]
pub enum StoredValue<T> {
    Text(SecretString),
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
        self.encrypt_text(text.expose_secret(), override_key)
            .map(|sealed| StoredValue::Text(SecretString::from(sealed)))
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
        let algorithm = self.settings.algorithm;
        let sealed = URL_SAFE.encode(seal(algorithm, key, &self.nonces, text.as_bytes(), b"")?);
        Ok(match algorithm {
            Algorithm::Aes256Gcm => format!("{AES_PREFIX}{sealed}"),
            Algorithm::XSalsa20Poly1305 => sealed,
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
            .and_then(|key| open_stored(text.expose_secret(), key));
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

fn open_stored(text: &str, key: &SecretString) -> Result<SecretString, SecretError> {
    if let Some(encoded) = text.strip_prefix(AES_PREFIX) {
        let sealed = decode_lenient(encoded, true)?;
        return open(Algorithm::Aes256Gcm, key, &sealed, b"");
    }
    open(Algorithm::XSalsa20Poly1305, key, &legacy_bytes(text)?, b"")
}

fn legacy_bytes(text: &str) -> Result<Vec<u8>, SecretError> {
    decode_lenient(text, true).or_else(|_url| decode_lenient(text, false))
}

#[must_use]
pub fn decrypt_if_encrypted_with(value: &str, key: &SecretString) -> Option<SecretString> {
    // "" and "*" decode to no bytes, which the legacy box reads as empty text under any key
    if !value.starts_with(AES_PREFIX) && legacy_bytes(value).ok()?.is_empty() {
        return None;
    }
    open_stored(value, key).ok()
}

mod tests;
