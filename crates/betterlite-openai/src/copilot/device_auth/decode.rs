use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;

pub const REPLY_LIMIT: usize = 0x0001_0000;

#[derive(Debug, Error)]
pub enum ReplyError {
    #[error("the GitHub reply is larger than {limit} bytes")]
    Size { limit: usize },
    #[error("the GitHub reply is not valid JSON")]
    Json(#[source] serde_json::Error),
}

#[derive(Debug, Default, Deserialize)]
pub struct DeviceReply {
    #[serde(default)]
    pub device_code: Option<String>,
    #[serde(default)]
    pub user_code: Option<String>,
    #[serde(default)]
    pub verification_uri: Option<String>,
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct AccessReply {
    #[serde(default)]
    pub access_token: Option<SecretString>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn decode_device_reply(bytes: &[u8]) -> Result<DeviceReply, ReplyError> {
    if bytes.len() > REPLY_LIMIT {
        return Err(ReplyError::Size { limit: REPLY_LIMIT });
    }
    serde_json::from_slice(bytes).map_err(ReplyError::Json)
}

pub fn decode_access_reply(bytes: &[u8]) -> Result<AccessReply, ReplyError> {
    if bytes.len() > REPLY_LIMIT {
        return Err(ReplyError::Size { limit: REPLY_LIMIT });
    }
    serde_json::from_slice(bytes).map_err(ReplyError::Json)
}
