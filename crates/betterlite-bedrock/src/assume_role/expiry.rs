use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeDelta, Utc};
use secrecy::SecretString;
use serde::Deserialize;

use super::error::ExpiryError;
use crate::aws_settings::sources::Credentials;

const EXPIRY_SAFETY_MARGIN: TimeDelta = TimeDelta::minutes(1);

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Expiration {
    Aware(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}

#[derive(Debug, Deserialize)]
pub struct StsCredentials {
    #[serde(rename = "AccessKeyId")]
    pub access_key_id: SecretString,
    #[serde(rename = "SecretAccessKey")]
    pub secret_access_key: SecretString,
    #[serde(rename = "SessionToken")]
    pub session_token: SecretString,
    #[serde(rename = "Expiration")]
    pub expiration: Expiration,
}

#[derive(Debug, Deserialize)]
pub struct StsResponse {
    #[serde(rename = "Credentials")]
    pub credentials: StsCredentials,
}

pub fn extract_credentials_and_ttl(
    response: StsResponse,
    now: DateTime<Utc>,
) -> Result<(Credentials, i64), ExpiryError> {
    let sts = response.credentials;
    let remaining = match sts.expiration {
        Expiration::Aware(expiry) => expiry.signed_duration_since(now),
        Expiration::Naive(expiry) => expiry.signed_duration_since(now.naive_utc()),
    };
    let ttl = remaining
        .checked_sub(&EXPIRY_SAFETY_MARGIN)
        .ok_or(ExpiryError::OutOfRange)?
        .num_seconds();
    let credentials = Credentials {
        access_key_id: sts.access_key_id,
        secret_access_key: sts.secret_access_key,
        session_token: Some(sts.session_token),
    };
    Ok((credentials, ttl))
}
