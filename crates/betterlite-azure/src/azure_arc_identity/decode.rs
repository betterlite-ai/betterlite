use std::str;

use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::error::Category;

use super::error::ArcIdentityError;

#[derive(Debug)]
pub struct ArcTokenReply {
    pub access_token: SecretString,
    pub expires_in: i64,
    pub token_type: String,
    pub resource: Option<String>,
}

#[derive(Debug)]
pub enum ArcReply {
    Token(ArcTokenReply),
    Rejected { access_token: Option<SecretString> },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Duration {
    Text(String),
    Whole(i64),
}

// The flattened map makes serde refuse a JSON array where an object belongs.
#[derive(Deserialize)]
struct RawReply {
    access_token: Option<String>,
    expires_in: Option<Duration>,
    token_type: Option<String>,
    resource: Option<String>,
    #[serde(flatten)]
    _other: IndexMap<String, IgnoredAny>,
}

#[must_use]
pub fn decode_arc_challenge(header: &[u8]) -> Option<String> {
    let text = str::from_utf8(header).ok()?;
    let (realm, key_name) = text.split_once('=')?;
    (!key_name.contains('=') && realm.to_lowercase() == "basic realm").then(|| key_name.to_owned())
}

fn parse_document(body: &[u8]) -> Result<Option<RawReply>, ArcIdentityError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    match serde_path_to_error::deserialize::<_, RawReply>(&mut deserializer) {
        Ok(raw) => Ok(deserializer.end().is_ok().then_some(raw)),
        Err(error) if error.inner().classify() == Category::Data => {
            Err(ArcIdentityError::ReplyValue {
                path: error.path().to_string(),
            })
        }
        Err(_) => Ok(None),
    }
}

fn expiry_seconds(duration: Option<Duration>) -> Result<Option<i64>, ArcIdentityError> {
    match duration {
        Some(Duration::Whole(seconds)) if seconds != 0 => Ok(Some(seconds)),
        Some(Duration::Text(text)) if !text.is_empty() => text
            .trim()
            .parse::<i64>()
            .ok()
            .map(Some)
            .ok_or_else(|| ArcIdentityError::ReplyValue {
                path: "expires_in".to_owned(),
            }),
        _ => Ok(None),
    }
}

pub fn decode_arc_identity_reply(body: &[u8]) -> Result<ArcReply, ArcIdentityError> {
    let Some(raw) = parse_document(body)? else {
        return Ok(ArcReply::Rejected { access_token: None });
    };
    let Some(token) = raw.access_token.filter(|token| !token.is_empty()) else {
        return Ok(ArcReply::Rejected { access_token: None });
    };
    let access_token = SecretString::from(token);
    match expiry_seconds(raw.expires_in)? {
        Some(expires_in) => Ok(ArcReply::Token(ArcTokenReply {
            access_token,
            expires_in,
            token_type: raw.token_type.unwrap_or_else(|| "Bearer".to_owned()),
            resource: raw.resource,
        })),
        None => Ok(ArcReply::Rejected {
            access_token: Some(access_token),
        }),
    }
}
