use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Number;

pub const REPLY_LIMIT: usize = 0x0010_0000;
const DEFAULT_INTERVAL: i64 = 5;
const POLL_WINDOW: i64 = 900;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("OAuth reply exceeds 1 MiB")]
    Size,
    #[error("invalid OAuth reply")]
    Json(#[source] serde_json::Error),
    #[error("OAuth reply is missing required fields")]
    Missing,
    #[error("device code interval is not a whole number of seconds")]
    Interval,
}

pub fn decode_chatgpt_oauth<Reply>(bytes: &[u8]) -> Result<Reply, DecodeError>
where
    Reply: DeserializeOwned,
{
    if bytes.len() > REPLY_LIMIT {
        return Err(DecodeError::Size);
    }
    serde_json::from_slice(bytes).map_err(DecodeError::Json)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Interval {
    Whole(Number),
    Text(String),
    Flag(bool),
}

#[derive(Deserialize)]
pub struct DeviceReply {
    device_auth_id: Option<SecretString>,
    user_code: Option<SecretString>,
    usercode: Option<SecretString>,
    interval: Option<Interval>,
}

pub struct DeviceCode {
    pub device_auth_id: SecretString,
    pub user_code: SecretString,
    pub interval: Duration,
}

impl DeviceReply {
    pub fn into_code(self) -> Result<DeviceCode, DecodeError> {
        let (Some(device_auth_id), Some(user_code)) = (
            self.device_auth_id
                .filter(|text| !text.expose_secret().is_empty()),
            self.user_code
                .filter(|text| !text.expose_secret().is_empty())
                .or_else(|| {
                    self.usercode
                        .filter(|text| !text.expose_secret().is_empty())
                }),
        ) else {
            return Err(DecodeError::Missing);
        };
        let seconds = match self.interval {
            None | Some(Interval::Flag(false)) => DEFAULT_INTERVAL,
            Some(Interval::Whole(number)) => number.as_i64().ok_or(DecodeError::Interval)?,
            Some(Interval::Text(text)) if text.is_empty() => DEFAULT_INTERVAL,
            Some(Interval::Text(text)) => text.trim().parse().ok().ok_or(DecodeError::Interval)?,
            Some(Interval::Flag(true)) => return Err(DecodeError::Interval),
        };
        // sleeping past the 900 second window ends the poll the same way as sleeping to its edge
        let bounded = seconds.clamp(DEFAULT_INTERVAL, POLL_WINDOW);
        Ok(DeviceCode {
            device_auth_id,
            user_code,
            interval: Duration::from_secs(u64::try_from(bounded).unwrap_or_default()),
        })
    }
}

#[derive(Deserialize)]
pub struct AuthorizationReply {
    authorization_code: Option<SecretString>,
    code_challenge: Option<SecretString>,
    code_verifier: Option<SecretString>,
}

pub struct AuthorizationCode {
    pub authorization_code: SecretString,
    pub code_verifier: SecretString,
}

impl AuthorizationReply {
    // a successful reply that lacks a field means the user has not finished yet
    #[must_use]
    pub fn complete(self) -> Option<AuthorizationCode> {
        match (
            self.authorization_code
                .filter(|text| !text.expose_secret().is_empty()),
            self.code_challenge
                .filter(|text| !text.expose_secret().is_empty()),
            self.code_verifier
                .filter(|text| !text.expose_secret().is_empty()),
        ) {
            (Some(authorization_code), Some(_), Some(code_verifier)) => Some(AuthorizationCode {
                authorization_code,
                code_verifier,
            }),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
pub struct TokenReply {
    #[serde(rename = "access_token")]
    access: Option<SecretString>,
    #[serde(rename = "refresh_token")]
    refresh: Option<SecretString>,
    #[serde(rename = "id_token")]
    id: Option<SecretString>,
}

pub struct Tokens {
    pub access_token: SecretString,
    pub refresh_token: SecretString,
    pub id_token: SecretString,
}

impl TokenReply {
    pub fn exchanged(self) -> Result<Tokens, DecodeError> {
        match (
            self.access.filter(|text| !text.expose_secret().is_empty()),
            self.refresh.filter(|text| !text.expose_secret().is_empty()),
            self.id.filter(|text| !text.expose_secret().is_empty()),
        ) {
            (Some(access_token), Some(refresh_token), Some(id_token)) => Ok(Tokens {
                access_token,
                refresh_token,
                id_token,
            }),
            _ => Err(DecodeError::Missing),
        }
    }

    pub fn refreshed(self, previous: SecretString) -> Result<Tokens, DecodeError> {
        match (
            self.access.filter(|text| !text.expose_secret().is_empty()),
            self.id.filter(|text| !text.expose_secret().is_empty()),
        ) {
            (Some(access_token), Some(id_token)) => Ok(Tokens {
                access_token,
                refresh_token: self
                    .refresh
                    .filter(|text| !text.expose_secret().is_empty())
                    .unwrap_or(previous),
                id_token,
            }),
            _ => Err(DecodeError::Missing),
        }
    }
}
