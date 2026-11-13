pub mod builders;
pub mod payload;
pub mod record;

mod tests;

use std::collections::BTreeMap;
use std::str::{Utf8Error, from_utf8};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_secrets::secret_codec::StoredCipher;
use betterlite_secrets::secret_codec::bearer::decrypt_bearer_token;
use secrecy::ExposeSecret as _;
use thiserror::Error;

// the prefix is also the AES-GCM AAD, so a token minted for one bearer kind never opens as another
pub const LITELLM_SESSION_TOKEN_PREFIX: &str = "litellm_login_";
pub const MAX_SESSION_TOKEN_BYTES: usize = 0x0010_0000;
pub const MAX_SESSION_JSON_DEPTH: usize = 64;

// the decrypted record before UserAPIKeyAuth validation, with duplicate keys already rejected
pub type TypedValue = BTreeMap<String, JsonData>;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("the session token is {length} bytes, over the {limit} byte limit")]
    TooLarge { length: usize, limit: usize },
    #[error("the decrypted session token is over the JSON depth limit of {limit}")]
    TooDeep { limit: usize },
    #[error("the session token is not valid UTF-8")]
    Utf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the session token is not a bearer token")]
    NotAToken,
    #[error("the decrypted session token is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the decrypted session token is not a JSON object")]
    NotAnObject,
}

pub fn decode_session_token<N, D>(
    cipher: &StoredCipher<N, D>,
    token: &[u8],
) -> Result<TypedValue, DecodeError> {
    if token.len() > MAX_SESSION_TOKEN_BYTES {
        return Err(DecodeError::TooLarge {
            length: token.len(),
            limit: MAX_SESSION_TOKEN_BYTES,
        });
    }
    let text = from_utf8(token).map_err(|source| DecodeError::Utf8 { source })?;
    let opened = decrypt_bearer_token(cipher, text, LITELLM_SESSION_TOKEN_PREFIX)
        .ok_or(DecodeError::NotAToken)?;
    let plaintext = opened.expose_secret();
    check_depth(plaintext)?;
    let value = serde_json::from_str::<JsonData>(plaintext)
        .map_err(|source| DecodeError::Json { source })?;
    match value {
        JsonData::Object(fields) => Ok(fields),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(DecodeError::NotAnObject),
    }
}

#[derive(Clone, Copy)]
enum ScanState {
    Structural,
    Quoted,
    Escaped,
}

fn check_depth(text: &str) -> Result<(), DecodeError> {
    let mut depth = 0_usize;
    let mut state = ScanState::Structural;
    for byte in text.bytes() {
        state = match (state, byte) {
            (ScanState::Structural, b'"') | (ScanState::Escaped, _) => ScanState::Quoted,
            (ScanState::Structural, b'{' | b'[') => {
                depth = depth.saturating_add(1);
                if depth > MAX_SESSION_JSON_DEPTH {
                    return Err(DecodeError::TooDeep {
                        limit: MAX_SESSION_JSON_DEPTH,
                    });
                }
                ScanState::Structural
            }
            (ScanState::Structural, b'}' | b']') => {
                depth = depth.saturating_sub(1);
                ScanState::Structural
            }
            (ScanState::Structural, _) | (ScanState::Quoted, b'"') => ScanState::Structural,
            (ScanState::Quoted, b'\\') => ScanState::Escaped,
            (ScanState::Quoted, _) => ScanState::Quoted,
        };
    }
    Ok(())
}
