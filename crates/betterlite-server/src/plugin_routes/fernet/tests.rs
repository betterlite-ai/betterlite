#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use cbc::cipher::block_padding::NoPadding;
use cbc::cipher::{BlockModeEncrypt as _, Iv, Key, KeyIvInit as _};

use super::{
    Aes128CbcEncryptor, FernetError, FernetKey, HEADER_BYTES, MINIMUM_TOKEN_BYTES, VERSION, open,
    seal,
};
use crate::plugin_routes::test_support::{
    CLAIM_IV, CLAIM_PLAINTEXT, CLAIM_TIMESTAMP, CLAIM_TOKEN, KEY_HEX,
};

fn python_key() -> FernetKey {
    let bytes = hex::decode(KEY_HEX).unwrap();
    let material: [u8; 32] = bytes.try_into().unwrap();
    FernetKey::from_material(material)
}

#[test]
fn opens_the_python_token() {
    let plaintext = open(&python_key(), CLAIM_TOKEN, 30, CLAIM_TIMESTAMP).unwrap();
    assert_eq!(plaintext, CLAIM_PLAINTEXT.as_bytes().to_vec());
}

#[test]
fn seals_the_python_token() {
    let token = seal(
        &python_key(),
        CLAIM_PLAINTEXT.as_bytes(),
        CLAIM_TIMESTAMP,
        CLAIM_IV,
    );
    assert_eq!(token, CLAIM_TOKEN);
}

#[test]
fn opens_the_python_token_at_the_expiry_second() {
    let plaintext = open(&python_key(), CLAIM_TOKEN, 30, CLAIM_TIMESTAMP + 30).unwrap();
    assert_eq!(plaintext, CLAIM_PLAINTEXT.as_bytes().to_vec());
}

#[test]
fn rejects_the_python_token_one_second_after_expiry() {
    assert_eq!(
        open(&python_key(), CLAIM_TOKEN, 30, CLAIM_TIMESTAMP + 31),
        Err(FernetError::Expired)
    );
}

#[test]
fn opens_a_token_at_the_clock_skew_second() {
    let plaintext = open(&python_key(), CLAIM_TOKEN, 30, CLAIM_TIMESTAMP - 60).unwrap();
    assert_eq!(plaintext, CLAIM_PLAINTEXT.as_bytes().to_vec());
}

#[test]
fn rejects_a_token_from_beyond_the_clock_skew() {
    assert_eq!(
        open(&python_key(), CLAIM_TOKEN, 30, CLAIM_TIMESTAMP - 61),
        Err(FernetError::Future)
    );
}

#[test]
fn rejects_a_tampered_ciphertext() {
    let mut raw = URL_SAFE.decode(CLAIM_TOKEN).unwrap();
    raw[HEADER_BYTES] ^= 0x01;
    let token = URL_SAFE.encode(raw);
    assert_eq!(
        open(&python_key(), &token, 30, CLAIM_TIMESTAMP),
        Err(FernetError::Signature)
    );
}

#[test]
fn rejects_a_tampered_tag() {
    let mut raw = URL_SAFE.decode(CLAIM_TOKEN).unwrap();
    let last = raw.len() - 1;
    raw[last] ^= 0x01;
    let token = URL_SAFE.encode(raw);
    assert_eq!(
        open(&python_key(), &token, 30, CLAIM_TIMESTAMP),
        Err(FernetError::Signature)
    );
}

#[test]
fn rejects_a_token_that_is_not_base64() {
    assert_eq!(
        open(&python_key(), "not base64!", 30, CLAIM_TIMESTAMP),
        Err(FernetError::Encoding)
    );
}

#[test]
fn rejects_another_token_version() {
    let mut raw = URL_SAFE.decode(CLAIM_TOKEN).unwrap();
    raw[0] = VERSION + 1;
    let token = URL_SAFE.encode(raw);
    assert_eq!(
        open(&python_key(), &token, 30, CLAIM_TIMESTAMP),
        Err(FernetError::Version)
    );
}

#[test]
fn rejects_a_token_shorter_than_the_minimum() {
    let token = URL_SAFE.encode([0_u8; 40]);
    assert_eq!(
        open(&python_key(), &token, 30, CLAIM_TIMESTAMP),
        Err(FernetError::Truncated)
    );
}

#[test]
fn accepts_a_token_at_the_minimum_length() {
    let token = seal(&python_key(), b"", CLAIM_TIMESTAMP, CLAIM_IV);
    assert_eq!(URL_SAFE.decode(&token).unwrap().len(), MINIMUM_TOKEN_BYTES);
    assert_eq!(
        open(&python_key(), &token, 30, CLAIM_TIMESTAMP).unwrap(),
        Vec::<u8>::new()
    );
}

#[test]
fn rejects_a_ciphertext_that_does_not_unpad() {
    let key = python_key();
    // encrypting zero bytes without padding decrypts to a block whose last byte is 0, never a pad length
    let ciphertext = Aes128CbcEncryptor::new(
        &Key::<Aes128CbcEncryptor>::from(key.encryption),
        &Iv::<Aes128CbcEncryptor>::from(CLAIM_IV),
    )
    .encrypt_padded_vec::<NoPadding>(&[0_u8; 16]);
    let mut raw = vec![VERSION];
    raw.extend_from_slice(&CLAIM_TIMESTAMP.to_be_bytes());
    raw.extend_from_slice(&CLAIM_IV);
    raw.extend_from_slice(&ciphertext);
    let tag = key.tag(&raw);
    raw.extend_from_slice(&tag);
    let token = URL_SAFE.encode(raw);
    assert_eq!(
        open(&key, &token, 30, CLAIM_TIMESTAMP),
        Err(FernetError::Ciphertext)
    );
}
