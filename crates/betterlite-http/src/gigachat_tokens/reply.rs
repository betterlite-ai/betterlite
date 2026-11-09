use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::gigachat_tokens::error::GigachatAuthError;

pub const MAX_REPLY_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub struct TokenReply {
    pub token: SecretString,
    pub expires_at: Option<i64>,
}

#[derive(Deserialize)]
struct ReplyBody {
    tok: Option<RawText>,
    access_token: Option<RawText>,
    exp: Option<Expiry>,
    expires_at: Option<Expiry>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawText {
    Text(String),
    Other(IgnoredAny),
}

impl RawText {
    fn non_empty(self) -> Option<String> {
        match self {
            Self::Text(text) if text.is_empty() => None,
            Self::Text(text) => Some(text),
            Self::Other(_) => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Expiry {
    Milliseconds(i64),
    Text(String),
}

pub fn decode_token_reply(bytes: &[u8]) -> Result<TokenReply, GigachatAuthError> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(GigachatAuthError::InvalidReply);
    }
    // A derived struct deserializes from a JSON array too, so the object form is checked first
    let starts_object = bytes
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| *byte == b'{');
    if !starts_object {
        return Err(GigachatAuthError::InvalidReply);
    }
    let Ok(reply) = serde_json::from_slice::<ReplyBody>(bytes) else {
        return Err(GigachatAuthError::InvalidReply);
    };
    let token = reply
        .tok
        .and_then(RawText::non_empty)
        .or_else(|| reply.access_token.and_then(RawText::non_empty))
        .ok_or(GigachatAuthError::InvalidReply)?;
    let expires_at = match expiry(reply.exp)? {
        Some(millis) => Some(millis),
        None => expiry(reply.expires_at)?,
    };
    Ok(TokenReply {
        token: SecretString::from(token),
        expires_at,
    })
}

fn expiry(value: Option<Expiry>) -> Result<Option<i64>, GigachatAuthError> {
    match value {
        None | Some(Expiry::Milliseconds(0)) => Ok(None),
        Some(Expiry::Milliseconds(millis)) => Ok(Some(millis)),
        Some(Expiry::Text(text)) if text.is_empty() => Ok(None),
        Some(Expiry::Text(text)) => {
            let Ok(millis) = text.trim().parse::<i64>() else {
                return Err(GigachatAuthError::InvalidReply);
            };
            Ok(Some(millis))
        }
    }
}
