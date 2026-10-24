use aes_gcm::aead::{Aead as _, Payload};
use aes_gcm::{Aes256Gcm, KeyInit as _, Nonce};
use crypto_secretbox::aead::{Aead as _, KeyInit as _};
use crypto_secretbox::{Key, XSalsa20Poly1305};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use crate::error::SecretError;
use crate::secret_codec::NonceSource;

const AES_NONCE: usize = 12;
const LEGACY_NONCE: usize = 24;

// LiteLLM keys both ciphers with one unsalted SHA-256; a real KDF would strand every stored row
pub(crate) fn seal_aes<N>(
    secret: &SecretString,
    nonces: &N,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, SecretError>
where
    N: NonceSource,
{
    let mut nonce = [0_u8; AES_NONCE];
    nonces.fill(&mut nonce)?;
    let cipher = Aes256Gcm::new(&Sha256::digest(secret.expose_secret().as_bytes()));
    let payload = Payload {
        msg: plaintext,
        aad,
    };
    let sealed = cipher
        .encrypt(&Nonce::from(nonce), payload)
        .map_err(|source| SecretError::Seal { source })?;
    Ok([nonce.as_slice(), &sealed].concat())
}

pub(crate) fn open_aes(
    secret: &SecretString,
    sealed: &[u8],
    aad: &[u8],
) -> Result<String, SecretError> {
    let (nonce, ciphertext) = sealed
        .split_first_chunk::<AES_NONCE>()
        .ok_or(SecretError::Truncated)?;
    let cipher = Aes256Gcm::new(&Sha256::digest(secret.expose_secret().as_bytes()));
    let payload = Payload {
        msg: ciphertext,
        aad,
    };
    let plaintext = cipher
        .decrypt(&Nonce::from(*nonce), payload)
        .map_err(|source| SecretError::Seal { source })?;
    String::from_utf8(plaintext).map_err(|error| SecretError::Utf8 {
        source: error.utf8_error(),
    })
}

pub(crate) fn seal_legacy<N>(
    secret: &SecretString,
    nonces: &N,
    plaintext: &[u8],
) -> Result<Vec<u8>, SecretError>
where
    N: NonceSource,
{
    let mut nonce = [0_u8; LEGACY_NONCE];
    nonces.fill(&mut nonce)?;
    let digest = <[u8; 32]>::from(Sha256::digest(secret.expose_secret().as_bytes()));
    let sealed = XSalsa20Poly1305::new(&Key::from(digest))
        .encrypt(&crypto_secretbox::Nonce::from(nonce), plaintext)
        .map_err(|source| SecretError::LegacySeal { source })?;
    Ok([nonce.as_slice(), &sealed].concat())
}

// PyNaCl reads an empty buffer as empty plaintext, before it looks for a nonce
pub(crate) fn open_legacy(secret: &SecretString, sealed: &[u8]) -> Result<String, SecretError> {
    let digest = <[u8; 32]>::from(Sha256::digest(secret.expose_secret().as_bytes()));
    if sealed.is_empty() {
        return Ok(String::new());
    }
    let (nonce, ciphertext) = sealed
        .split_first_chunk::<LEGACY_NONCE>()
        .ok_or(SecretError::Truncated)?;
    let plaintext = XSalsa20Poly1305::new(&Key::from(digest))
        .decrypt(&crypto_secretbox::Nonce::from(*nonce), ciphertext)
        .map_err(|source| SecretError::LegacySeal { source })?;
    String::from_utf8(plaintext).map_err(|error| SecretError::Utf8 {
        source: error.utf8_error(),
    })
}
