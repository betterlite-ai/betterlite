use aes::Aes128;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt as _, BlockModeEncrypt as _, Iv, Key, KeyIvInit as _};
use cbc::{Decryptor, Encryptor};
use hmac::digest::Key as HmacKey;
use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;
use subtle::ConstantTimeEq as _;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;
type Aes128CbcEncryptor = Encryptor<Aes128>;
type Aes128CbcDecryptor = Decryptor<Aes128>;

const KEY_BYTES: usize = 32;
const HALF_KEY_BYTES: usize = 16;
const HMAC_BYTES: usize = 32;
const HMAC_BLOCK_BYTES: usize = 64;
pub(crate) const IV_BYTES: usize = 16;
const VERSION: u8 = 0x80;
const TIMESTAMP_BYTES: usize = 8;
const HEADER_BYTES: usize = 1 + TIMESTAMP_BYTES + IV_BYTES;
const BLOCK_BYTES: usize = 16;
const MINIMUM_TOKEN_BYTES: usize = HEADER_BYTES + HMAC_BYTES + BLOCK_BYTES;
const CLOCK_SKEW_SECONDS: u64 = 60;

#[derive(Debug, Eq, PartialEq, Error)]
pub(crate) enum FernetError {
    #[error("the token is not base64url text")]
    Encoding,
    #[error("the token version is not 0x80")]
    Version,
    #[error("the token is shorter than a Fernet token")]
    Truncated,
    #[error("the token is older than its time to live")]
    Expired,
    #[error("the token timestamp is too far in the future")]
    Future,
    #[error("the token signature does not match")]
    Signature,
    #[error("the token ciphertext does not decrypt")]
    Ciphertext,
}

#[derive(Clone)]
pub(crate) struct FernetKey {
    signing: [u8; HALF_KEY_BYTES],
    encryption: [u8; HALF_KEY_BYTES],
}

impl FernetKey {
    // Split as the spec does: the first half signs, the second half encrypts.
    pub(crate) fn from_material(material: [u8; KEY_BYTES]) -> Self {
        let mut signing = [0_u8; HALF_KEY_BYTES];
        let mut encryption = [0_u8; HALF_KEY_BYTES];
        let (first, second) = material.split_at(HALF_KEY_BYTES);
        signing.copy_from_slice(first);
        encryption.copy_from_slice(second);
        Self {
            signing,
            encryption,
        }
    }

    fn tag(&self, message: &[u8]) -> [u8; HMAC_BYTES] {
        // a Fernet signing key is the 16-byte half zero-padded to the hash block, as hmac.new pads it
        let mut padded = [0_u8; HMAC_BLOCK_BYTES];
        let (head, _tail) = padded.split_at_mut(HALF_KEY_BYTES);
        head.copy_from_slice(&self.signing);
        let mut mac = HmacSha256::new(&HmacKey::<HmacSha256>::from(padded));
        mac.update(message);
        <[u8; HMAC_BYTES]>::from(mac.finalize().into_bytes())
    }
}

pub(crate) fn seal(
    key: &FernetKey,
    plaintext: &[u8],
    timestamp: u64,
    iv: [u8; IV_BYTES],
) -> String {
    let encrypted = Aes128CbcEncryptor::new(
        &Key::<Aes128CbcEncryptor>::from(key.encryption),
        &Iv::<Aes128CbcEncryptor>::from(iv),
    )
    .encrypt_padded_vec::<Pkcs7>(plaintext);
    let mut token = Vec::new();
    token.push(VERSION);
    token.extend_from_slice(&timestamp.to_be_bytes());
    token.extend_from_slice(&iv);
    token.extend_from_slice(&encrypted);
    let tag = key.tag(&token);
    token.extend_from_slice(&tag);
    URL_SAFE.encode(token)
}

pub(crate) fn open(
    key: &FernetKey,
    token: &str,
    ttl: u64,
    now: u64,
) -> Result<Vec<u8>, FernetError> {
    let raw = URL_SAFE
        .decode(token)
        .map_err(|_error| FernetError::Encoding)?;
    if raw.len() < MINIMUM_TOKEN_BYTES {
        return Err(FernetError::Truncated);
    }
    let (after_timestamp, timestamp) = version_and_timestamp(&raw)?;
    // cryptography checks the version, then the time to live, and only then the signature
    if timestamp.saturating_add(ttl) < now {
        return Err(FernetError::Expired);
    }
    if now.saturating_add(CLOCK_SKEW_SECONDS) < timestamp {
        return Err(FernetError::Future);
    }
    let (signed, signature) = split_signature(&raw)?;
    let expected = key.tag(signed);
    if !bool::from(expected.as_slice().ct_eq(signature)) {
        return Err(FernetError::Signature);
    }
    let (ciphertext, iv) = body_and_iv(after_timestamp)?;
    Aes128CbcDecryptor::new(
        &Key::<Aes128CbcDecryptor>::from(key.encryption),
        &Iv::<Aes128CbcDecryptor>::from(iv),
    )
    .decrypt_padded_vec::<Pkcs7>(ciphertext)
    .map_err(|_error| FernetError::Ciphertext)
}

fn version_and_timestamp(raw: &[u8]) -> Result<(&[u8], u64), FernetError> {
    let (version, after_version) = raw.split_first().ok_or(FernetError::Truncated)?;
    if *version != VERSION {
        return Err(FernetError::Version);
    }
    let (timestamp_field, after_timestamp) = after_version
        .split_at_checked(TIMESTAMP_BYTES)
        .ok_or(FernetError::Truncated)?;
    let timestamp = u64::from_be_bytes(
        timestamp_field
            .try_into()
            .map_err(|_error| FernetError::Truncated)?,
    );
    Ok((after_timestamp, timestamp))
}

fn split_signature(raw: &[u8]) -> Result<(&[u8], &[u8]), FernetError> {
    let signed_length = raw
        .len()
        .checked_sub(HMAC_BYTES)
        .ok_or(FernetError::Truncated)?;
    raw.split_at_checked(signed_length)
        .ok_or(FernetError::Truncated)
}

fn body_and_iv(after_timestamp: &[u8]) -> Result<(&[u8], [u8; IV_BYTES]), FernetError> {
    let (iv_field, after_iv) = after_timestamp
        .split_at_checked(IV_BYTES)
        .ok_or(FernetError::Truncated)?;
    let ciphertext_length = after_iv
        .len()
        .checked_sub(HMAC_BYTES)
        .ok_or(FernetError::Truncated)?;
    let ciphertext = after_iv
        .get(..ciphertext_length)
        .ok_or(FernetError::Truncated)?;
    let iv = iv_field
        .try_into()
        .map_err(|_error| FernetError::Truncated)?;
    Ok((ciphertext, iv))
}

mod tests;
