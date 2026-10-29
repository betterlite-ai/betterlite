use aes_gcm::aead::AeadInOut as _;
use aes_gcm::aead::inout::InOutBuf;
use aes_gcm::{Aes256Gcm, KeyInit as _, Tag};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_redact::structured::Payload as Json;
use secrecy::{ExposeSecret as _, SecretSlice};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::error::SecretError;
use crate::secret_codec::NonceSource;

const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(Debug, Error)]
pub enum AppCryptoError {
    #[error("the master key is not a valid 32-byte AES-256 key")]
    KeyLength,
    #[error("could not draw a random nonce")]
    Random {
        #[source]
        source: SecretError,
    },
    #[error("could not encode the payload as JSON")]
    JsonEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the envelope failed authentication")]
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

pub struct AppCrypto<N> {
    cipher: Aes256Gcm,
    nonces: N,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub nonce: String,
    pub ciphertext: String,
    pub tag: String,
}

impl<N> AppCrypto<N>
where
    N: NonceSource,
{
    pub fn new(master_key: &SecretSlice<u8>, nonces: N) -> Result<Self, AppCryptoError> {
        let cipher = Aes256Gcm::new_from_slice(master_key.expose_secret())
            .map_err(|_error| AppCryptoError::KeyLength)?;
        Ok(Self { cipher, nonces })
    }

    pub fn encrypt_json(&self, data: &Json, aad: &[u8]) -> Result<Envelope, AppCryptoError> {
        let mut nonce = [0_u8; NONCE_LEN];
        self.nonces
            .fill(&mut nonce)
            .map_err(|source| AppCryptoError::Random { source })?;
        let buffer =
            serde_json::to_vec(data).map_err(|source| AppCryptoError::JsonEncode { source })?;
        seal_envelope(&self.cipher, nonce, aad, buffer)
    }

    pub fn decrypt_json(&self, envelope: &Envelope, aad: &[u8]) -> Result<Json, AppCryptoError> {
        let (nonce, mut buffer, tag) = decode_envelope(envelope)?;
        let aes_nonce = aes_gcm::Nonce::from(nonce);
        self.cipher
            .decrypt_inout_detached(&aes_nonce, aad, InOutBuf::from(&mut *buffer), &tag)
            .map_err(|source| AppCryptoError::Seal { source })?;
        serde_json::from_slice(&buffer).map_err(|source| AppCryptoError::JsonDecode { source })
    }
}

fn seal_envelope(
    cipher: &Aes256Gcm,
    nonce: [u8; NONCE_LEN],
    aad: &[u8],
    mut buffer: Vec<u8>,
) -> Result<Envelope, AppCryptoError> {
    let aes_nonce = aes_gcm::Nonce::from(nonce);
    let tag = cipher
        .encrypt_inout_detached(&aes_nonce, aad, InOutBuf::from(&mut *buffer))
        .map_err(|source| AppCryptoError::Seal { source })?;
    Ok(Envelope {
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(&buffer),
        tag: STANDARD.encode(tag),
    })
}

fn decode_envelope(envelope: &Envelope) -> Result<([u8; NONCE_LEN], Vec<u8>, Tag), AppCryptoError> {
    let nonce_bytes = STANDARD
        .decode(&envelope.nonce)
        .map_err(|source| AppCryptoError::Decode { source })?;
    let nonce: [u8; NONCE_LEN] = nonce_bytes
        .try_into()
        .map_err(|_bytes| AppCryptoError::Truncated)?;
    let ciphertext = STANDARD
        .decode(&envelope.ciphertext)
        .map_err(|source| AppCryptoError::Decode { source })?;
    let tag_bytes = STANDARD
        .decode(&envelope.tag)
        .map_err(|source| AppCryptoError::Decode { source })?;
    let tag_array: [u8; TAG_LEN] = tag_bytes
        .try_into()
        .map_err(|_bytes| AppCryptoError::Truncated)?;
    Ok((nonce, ciphertext, Tag::from(tag_array)))
}
