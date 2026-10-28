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

use crate::error::SecretError;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

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
    pub fn new(master_key: &SecretString) -> Result<Self, SecretError> {
        let bytes = master_key.expose_secret().as_bytes();
        if bytes.len() != KEY_LEN {
            return Err(SecretError::KeyLength {
                length: bytes.len(),
                expected: KEY_LEN,
            });
        }
        let key =
            aes_gcm::Key::<Aes256Gcm>::try_from(bytes).map_err(|_slice| SecretError::Truncated)?;
        Ok(Self {
            cipher: Aes256Gcm::new(&key),
        })
    }

    pub fn encrypt_json(&self, data: &Value, aad: Option<&[u8]>) -> Result<Envelope, SecretError> {
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|source| SecretError::Random { source })?;
        let plaintext =
            serde_json::to_vec(data).map_err(|source| SecretError::JsonEncode { source })?;
        let payload = Payload {
            msg: &plaintext,
            aad: aad.unwrap_or(b""),
        };
        let sealed = self
            .cipher
            .encrypt(&aes_gcm::Nonce::from(nonce), payload)
            .map_err(|source| SecretError::Seal { source })?;
        let (ciphertext, tag) = sealed
            .split_at_checked(sealed.len().saturating_sub(TAG_LEN))
            .ok_or(SecretError::Truncated)?;
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
    ) -> Result<Value, SecretError> {
        let nonce_bytes = STANDARD
            .decode(&envelope.nonce)
            .map_err(|source| SecretError::Decode { source })?;
        let nonce: [u8; NONCE_LEN] = nonce_bytes
            .try_into()
            .map_err(|_bytes| SecretError::Truncated)?;
        let ciphertext = STANDARD
            .decode(&envelope.ciphertext)
            .map_err(|source| SecretError::Decode { source })?;
        let tag = STANDARD
            .decode(&envelope.tag)
            .map_err(|source| SecretError::Decode { source })?;
        let sealed: Vec<u8> = ciphertext.into_iter().chain(tag).collect();
        let payload = Payload {
            msg: &sealed,
            aad: aad.unwrap_or(b""),
        };
        let plaintext = self
            .cipher
            .decrypt(&aes_gcm::Nonce::from(nonce), payload)
            .map_err(|source| SecretError::Seal { source })?;
        serde_json::from_slice(&plaintext).map_err(|source| SecretError::JsonDecode { source })
    }
}
