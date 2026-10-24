use aes_gcm::aead::{Aead as _, Payload};
use aes_gcm::{Aes256Gcm, KeyInit as _};
use crypto_secretbox::XSalsa20Poly1305;
use crypto_secretbox::aead::{Aead as _, KeyInit as _};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use crate::error::SecretError;
use crate::secret_codec::{Algorithm, NonceSource};

const AES_NONCE: usize = 12;
const LEGACY_NONCE: usize = 24;

// LiteLLM keys both ciphers with one unsalted SHA-256; a real KDF would strand every stored row
pub(crate) fn seal<N>(
    algorithm: Algorithm,
    secret: &SecretString,
    nonces: &N,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, SecretError>
where
    N: NonceSource,
{
    let digest = Sha256::digest(secret.expose_secret().as_bytes());
    let mut nonce = [0_u8; LEGACY_NONCE];
    let (used, sealed) = match algorithm {
        Algorithm::Aes256Gcm => {
            let (aes_nonce, _) = nonce
                .split_first_chunk_mut::<AES_NONCE>()
                .ok_or(SecretError::Truncated)?;
            nonces.fill(aes_nonce)?;
            let payload = Payload {
                msg: plaintext,
                aad,
            };
            let sealed =
                Aes256Gcm::new(&digest).encrypt(&aes_gcm::Nonce::from(*aes_nonce), payload);
            (
                AES_NONCE,
                sealed.map_err(|source| SecretError::Seal { source })?,
            )
        }
        Algorithm::XSalsa20Poly1305 => {
            nonces.fill(&mut nonce)?;
            let key = crypto_secretbox::Key::from(<[u8; 32]>::from(digest));
            let sealed = XSalsa20Poly1305::new(&key)
                .encrypt(&crypto_secretbox::Nonce::from(nonce), plaintext);
            (
                LEGACY_NONCE,
                sealed.map_err(|source| SecretError::LegacySeal { source })?,
            )
        }
    };
    let prefix = nonce.get(..used).ok_or(SecretError::Truncated)?;
    Ok([prefix, &sealed].concat())
}

// LiteLLM's encrypt_value: raw SecretBox bytes, nonce then MAC and ciphertext, for callers that pick their own key
pub fn seal_legacy<N>(
    plaintext: &SecretString,
    signing_key: &SecretString,
    nonces: &N,
) -> Result<Vec<u8>, SecretError>
where
    N: NonceSource,
{
    let bytes = plaintext.expose_secret().as_bytes();
    seal(Algorithm::XSalsa20Poly1305, signing_key, nonces, bytes, b"")
}

pub fn open_legacy(sealed: &[u8], signing_key: &SecretString) -> Result<SecretString, SecretError> {
    open(Algorithm::XSalsa20Poly1305, signing_key, sealed, b"")
}

// PyNaCl reads an empty legacy buffer as empty plaintext, before it looks for a nonce
pub(crate) fn open(
    algorithm: Algorithm,
    secret: &SecretString,
    sealed: &[u8],
    aad: &[u8],
) -> Result<SecretString, SecretError> {
    let digest = Sha256::digest(secret.expose_secret().as_bytes());
    let plaintext = match algorithm {
        Algorithm::XSalsa20Poly1305 if sealed.is_empty() => Vec::new(),
        Algorithm::Aes256Gcm => {
            let (nonce, msg) = sealed
                .split_first_chunk::<AES_NONCE>()
                .ok_or(SecretError::Truncated)?;
            let payload = Payload { msg, aad };
            let opened = Aes256Gcm::new(&digest).decrypt(&aes_gcm::Nonce::from(*nonce), payload);
            opened.map_err(|source| SecretError::Seal { source })?
        }
        Algorithm::XSalsa20Poly1305 => {
            let (nonce, ciphertext) = sealed
                .split_first_chunk::<LEGACY_NONCE>()
                .ok_or(SecretError::Truncated)?;
            let key = crypto_secretbox::Key::from(<[u8; 32]>::from(digest));
            let opened = XSalsa20Poly1305::new(&key)
                .decrypt(&crypto_secretbox::Nonce::from(*nonce), ciphertext);
            opened.map_err(|source| SecretError::LegacySeal { source })?
        }
    };
    String::from_utf8(plaintext)
        .map(SecretString::from)
        .map_err(|error| SecretError::Utf8 {
            source: error.utf8_error(),
        })
}
