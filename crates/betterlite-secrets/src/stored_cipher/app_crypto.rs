use aes_gcm::aead::{Aead as _, Payload as AeadPayload};
use aes_gcm::{Aes256Gcm, KeyInit as _};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_redact::structured::Payload as Json;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

// Both directions seal the same way under the hood; only which AEAD call runs differs.
#[derive(Clone, Copy)]
enum Direction {
    Encrypt,
    Decrypt,
}

// AppCrypto's own envelope errors, distinct from the stored-value codec's SecretError.
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
    // cryptography's InvalidTag carries no message of its own; str(error) is "".
    #[error("")]
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

// Port of app_crypto.AppCrypto: an exact 32-byte key, no SHA-256 derivation.
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
        data: &Json,
        aad: Option<&[u8]>,
    ) -> Result<Envelope, AppCryptoError> {
        let mut nonce = [0_u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|source| AppCryptoError::Random { source })?;
        let plaintext =
            serde_json::to_vec(data).map_err(|source| AppCryptoError::JsonEncode { source })?;
        let sealed = self.transcode(nonce, &plaintext, aad, Direction::Encrypt)?;
        encode_envelope(nonce, &sealed)
    }

    pub fn decrypt_json(
        &self,
        envelope: &Envelope,
        aad: Option<&[u8]>,
    ) -> Result<Json, AppCryptoError> {
        let (nonce, sealed) = decode_envelope(envelope)?;
        let plaintext = self.transcode(nonce, &sealed, aad, Direction::Decrypt)?;
        serde_json::from_slice(&plaintext).map_err(|source| AppCryptoError::JsonDecode { source })
    }

    fn transcode(
        &self,
        nonce: [u8; NONCE_LEN],
        msg: &[u8],
        aad: Option<&[u8]>,
        direction: Direction,
    ) -> Result<Vec<u8>, AppCryptoError> {
        let aes_nonce = aes_gcm::Nonce::from(nonce);
        let payload = aead_payload(msg, aad);
        let sealed = match direction {
            Direction::Encrypt => self.cipher.encrypt(&aes_nonce, payload),
            Direction::Decrypt => self.cipher.decrypt(&aes_nonce, payload),
        };
        sealed.map_err(|source| AppCryptoError::Seal { source })
    }
}

fn aead_payload<'input>(
    msg: &'input [u8],
    aad: Option<&'input [u8]>,
) -> AeadPayload<'input, 'input> {
    let aad_bytes = aad.unwrap_or(b"");
    AeadPayload {
        msg,
        aad: aad_bytes,
    }
}

fn encode_envelope(nonce: [u8; NONCE_LEN], sealed: &[u8]) -> Result<Envelope, AppCryptoError> {
    let split_at = sealed.len().saturating_sub(TAG_LEN);
    let (ciphertext, tag) = sealed
        .split_at_checked(split_at)
        .ok_or(AppCryptoError::Truncated)?;
    Ok(Envelope {
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
        tag: STANDARD.encode(tag),
    })
}

fn decode_envelope(envelope: &Envelope) -> Result<([u8; NONCE_LEN], Vec<u8>), AppCryptoError> {
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
    Ok((nonce, sealed))
}
