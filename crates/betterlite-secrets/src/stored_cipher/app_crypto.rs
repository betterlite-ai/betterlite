#![expect(
    clippy::disallowed_types,
    reason = "AppCrypto's envelope carries an arbitrary caller JSON payload (the SQS integration), with no app-specific shape of its own"
)]

use aes_gcm::aead::{Aead as _, Payload};
use aes_gcm::{Aes256Gcm, KeyInit as _};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// `AppCrypto`'s own envelope errors.
///
/// Distinct from the stored-value codec's `SecretError`: a raw 32-byte key
/// and a JSON payload are a different failure surface from the
/// signing-key-derived scalar and map formats.
#[derive(Debug, Error)]
pub enum AppCryptoError {
    #[error("the master key is {length} bytes, not the required {expected}")]
    KeyLength { length: usize, expected: usize },
    #[error("could not draw a random nonce")]
    Random {
        #[source]
        source: getrandom::Error,
    },
    #[error("could not encode the payload as JSON")]
    JsonEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the ciphertext failed authentication under this key")]
    Seal {
        #[source]
        source: aes_gcm::Error,
    },
    #[error("the envelope field is not valid base64")]
    Decode {
        #[source]
        source: base64::DecodeError,
    },
    #[error("the envelope is too short to hold its nonce or tag")]
    Truncated,
    #[error("could not decode the payload as JSON")]
    JsonDecode {
        #[source]
        source: serde_json::Error,
    },
}

/// Port of `app_crypto.AppCrypto`.
///
/// AES-256-GCM over an exact 32-byte key with no SHA-256 derivation, and a
/// nonce/ciphertext/tag envelope in standard (not URL-safe) base64 — the
/// format the SQS integration expects.
pub struct AppCrypto {
    cipher: Aes256Gcm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub nonce: String,
    pub ciphertext: String,
    pub tag: String,
}

impl AppCrypto {
    pub fn new(master_key: &SecretString) -> Result<Self, AppCryptoError> {
        let bytes = master_key.expose_secret().as_bytes();
        if bytes.len() != KEY_LEN {
            return Err(AppCryptoError::KeyLength {
                length: bytes.len(),
                expected: KEY_LEN,
            });
        }
        let key = aes_gcm::Key::<Aes256Gcm>::try_from(bytes)
            .map_err(|_slice| AppCryptoError::Truncated)?;
        Ok(Self {
            cipher: Aes256Gcm::new(&key),
        })
    }

    pub fn encrypt_json(
        &self,
        data: &Value,
        aad: Option<&[u8]>,
    ) -> Result<Envelope, AppCryptoError> {
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|source| AppCryptoError::Random { source })?;
        let plaintext =
            serde_json::to_vec(data).map_err(|source| AppCryptoError::JsonEncode { source })?;
        let payload = Payload {
            msg: &plaintext,
            aad: aad.unwrap_or(b""),
        };
        let sealed = self
            .cipher
            .encrypt(&aes_gcm::Nonce::from(nonce), payload)
            .map_err(|source| AppCryptoError::Seal { source })?;
        let (ciphertext, tag) = sealed
            .split_at_checked(sealed.len().saturating_sub(TAG_LEN))
            .ok_or(AppCryptoError::Truncated)?;
        Ok(Envelope {
            nonce: STANDARD.encode(nonce),
            ciphertext: STANDARD.encode(ciphertext),
            tag: STANDARD.encode(tag),
        })
    }

    pub fn decrypt_json(
        &self,
        envelope: &Envelope,
        aad: Option<&[u8]>,
    ) -> Result<Value, AppCryptoError> {
        let nonce_bytes = STANDARD
            .decode(&envelope.nonce)
            .map_err(|source| AppCryptoError::Decode { source })?;
        let nonce: [u8; NONCE_LEN] = nonce_bytes
            .try_into()
            .map_err(|_bytes| AppCryptoError::Truncated)?;
        let ciphertext = STANDARD
            .decode(&envelope.ciphertext)
            .map_err(|source| AppCryptoError::Decode { source })?;
        let tag = STANDARD
            .decode(&envelope.tag)
            .map_err(|source| AppCryptoError::Decode { source })?;
        let sealed: Vec<u8> = ciphertext.into_iter().chain(tag).collect();
        let payload = Payload {
            msg: &sealed,
            aad: aad.unwrap_or(b""),
        };
        let plaintext = self
            .cipher
            .decrypt(&aes_gcm::Nonce::from(nonce), payload)
            .map_err(|source| AppCryptoError::Seal { source })?;
        serde_json::from_slice(&plaintext).map_err(|source| AppCryptoError::JsonDecode { source })
    }
}
