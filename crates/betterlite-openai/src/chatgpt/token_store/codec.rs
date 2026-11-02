use std::collections::BTreeMap;
use std::fmt;

use super::member::{ExposedSecret, ExtraValue, Member, PresentMember};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_with::{FromInto, FromIntoRef, serde_as};

pub const RECORD_LIMIT: usize = 0x0010_0000;
const TOKEN_LIMIT: usize = 0x4000;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("auth record exceeds 1 MiB")]
    Size,
    #[error("invalid auth record")]
    Json(#[source] serde_json::Error),
}

#[serde_as]
#[derive(Default, Clone, Deserialize, Serialize)]
pub struct AuthRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(
        serialize_as = "Option<FromIntoRef<Member<ExposedSecret>>>",
        deserialize_as = "FromInto<PresentMember<SecretString>>"
    )]
    pub access_token: Option<Member<SecretString>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(
        serialize_as = "Option<FromIntoRef<Member<ExposedSecret>>>",
        deserialize_as = "FromInto<PresentMember<SecretString>>"
    )]
    pub refresh_token: Option<Member<SecretString>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(
        serialize_as = "Option<FromIntoRef<Member<ExposedSecret>>>",
        deserialize_as = "FromInto<PresentMember<SecretString>>"
    )]
    pub id_token: Option<Member<SecretString>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(deserialize_as = "FromInto<PresentMember<Number>>")]
    pub expires_at: Option<Member<Number>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(deserialize_as = "FromInto<PresentMember<String>>")]
    pub account_id: Option<Member<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(deserialize_as = "FromInto<PresentMember<Number>>")]
    pub device_code_requested_at: Option<Member<Number>>,
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, ExtraValue>,
}

impl fmt::Debug for AuthRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthRecord").finish_non_exhaustive()
    }
}

pub fn decode_auth_record(bytes: &[u8]) -> Result<AuthRecord, DecodeError> {
    if bytes.len() > RECORD_LIMIT {
        return Err(DecodeError::Size);
    }
    serde_json::from_slice(bytes).map_err(DecodeError::Json)
}

#[derive(Debug, Default, Deserialize)]
pub struct TokenClaims {
    pub exp: Option<Member<Number>>,
    #[serde(rename = "https://api.openai.com/auth")]
    pub auth: Option<Member<AccountClaim>>,
}

#[derive(Debug, Deserialize)]
pub struct AccountClaim {
    pub chatgpt_account_id: Option<Member<String>>,
}

#[must_use]
pub fn decode_token_claims(bytes: &[u8]) -> Option<TokenClaims> {
    if bytes.len() > TOKEN_LIMIT {
        return None;
    }
    let payload = bytes.split(|byte| *byte == b'.').nth(1)?;
    let trimmed = payload.trim_ascii_end();
    let unpadded = trimmed
        .strip_suffix(b"==")
        .or_else(|| trimmed.strip_suffix(b"="))
        .unwrap_or(trimmed);
    let decoded = URL_SAFE_NO_PAD.decode(unpadded).ok()?;
    serde_json::from_slice(&decoded).ok()
}

#[must_use]
pub fn expires_at(token: &SecretString) -> Option<Number> {
    let claims = decode_token_claims(token.expose_secret().as_bytes())?;
    let exp = claims.exp?;
    let value = exp.known()?;
    if !value.as_str().contains(['.', 'e', 'E']) {
        return Some(value.clone());
    }
    let seconds = value.as_f64()?.trunc();
    if seconds == 0.0_f64 {
        Some(Number::from(0_i32))
    } else {
        seconds.to_string().parse().ok()
    }
}

#[must_use]
pub fn account_id(record: &AuthRecord) -> Option<String> {
    let token = record
        .id_token
        .as_ref()
        .filter(|member| match member {
            Member::Known(token) => !token.expose_secret().is_empty(),
            Member::Other(value) => !value.is_null(),
        })
        .or(record.access_token.as_ref())?;
    let claims = decode_token_claims(token.known()?.expose_secret().as_bytes())?;
    claims
        .auth?
        .known()?
        .chatgpt_account_id
        .as_ref()?
        .known()
        .filter(|value| !value.is_empty())
        .cloned()
}

#[must_use]
pub fn build_auth_record(mut tokens: AuthRecord) -> AuthRecord {
    tokens.expires_at = tokens
        .access_token
        .as_ref()
        .and_then(Member::known)
        .and_then(expires_at)
        .map(Member::Known);
    tokens.account_id = account_id(&tokens).map(Member::Known);
    tokens.additional_fields.clear();
    tokens.device_code_requested_at = None;
    tokens
        .access_token
        .get_or_insert(Member::Other(serde_json::json!(null)));
    tokens
        .refresh_token
        .get_or_insert(Member::Other(serde_json::json!(null)));
    tokens
        .id_token
        .get_or_insert(Member::Other(serde_json::json!(null)));
    tokens
        .expires_at
        .get_or_insert(Member::Other(serde_json::json!(null)));
    tokens
        .account_id
        .get_or_insert(Member::Other(serde_json::json!(null)));
    tokens
}
