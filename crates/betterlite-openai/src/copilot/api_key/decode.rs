use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_with::{FromIntoRef, serde_as};

use crate::copilot::device_auth::decode::{REPLY_LIMIT, ReplyError};

#[serde_as]
#[derive(Default, Deserialize, Serialize)]
pub struct CopilotKeyRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(serialize_as = "Option<FromIntoRef<ExposedToken>>")]
    pub token: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoints: Option<Endpoints>,
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize)]
#[serde(transparent)]
struct ExposedToken(String);

impl From<&SecretString> for ExposedToken {
    fn from(token: &SecretString) -> Self {
        Self(token.expose_secret().to_owned())
    }
}

#[derive(Default, Deserialize, Serialize)]
pub struct Endpoints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, serde_json::Value>,
}

pub fn decode_copilot_key_record(bytes: &[u8]) -> Result<CopilotKeyRecord, ReplyError> {
    if bytes.len() > REPLY_LIMIT {
        return Err(ReplyError::Size { limit: REPLY_LIMIT });
    }
    serde_json::from_slice(bytes).map_err(ReplyError::Json)
}

mod tests;
