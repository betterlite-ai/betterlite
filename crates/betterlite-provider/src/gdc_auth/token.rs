use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

pub const MAX_REPLY_BYTES: usize = 0x0001_0000;

#[rustfmt::skip]
#[derive(Debug)]
pub struct TokenReply { pub token: SecretString, pub expires_in: Option<i64> }
#[rustfmt::skip]
#[derive(Debug, Error)]
pub enum TokenReplyError {
    #[error("the token reply is larger than {limit} bytes")] TooLarge { limit: usize },
    #[error("the token reply is not the JSON object the token endpoint documents")] Body,
    #[error("the token reply expiry is not a whole number of seconds")] Expiry,
}
#[rustfmt::skip]
#[derive(Deserialize)]
struct ReplyBody { access_token: SecretString, #[serde(default)] expires_in: Option<Expiry> }
#[rustfmt::skip]
#[derive(Deserialize)] #[serde(untagged)]
enum Expiry { Seconds(i64), Text(String) }

#[rustfmt::skip]
pub fn decode_gdc_token_reply(bytes: &[u8]) -> Result<TokenReply, TokenReplyError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(TokenReplyError::TooLarge { limit: MAX_REPLY_BYTES });
    }
    // serde_json reads a derived struct from a JSON array, so the object form is checked first
    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()).is_none_or(|byte| *byte != b'{') {
        return Err(TokenReplyError::Body);
    }
    let reply = serde_json::from_slice::<ReplyBody>(bytes).map_err(|_error| TokenReplyError::Body)?;
    let expires_in = match reply.expires_in {
        None => None,
        Some(Expiry::Seconds(seconds)) => Some(seconds),
        Some(Expiry::Text(text)) => Some(text.trim().parse::<i64>().map_err(|_error| TokenReplyError::Expiry)?),
    };
    Ok(TokenReply { token: reply.access_token, expires_in })
}
