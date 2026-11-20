use std::time::Duration;

use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::jwt_claims::decode::unverified_claims;
use chrono::{DateTime, Datelike as _, NaiveDateTime, ParseError, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod tests;

const UNDECODABLE_WARNING: &str = "SSO id_token could not be decoded or its claims were unusable; not retaining it for EMA egress.";
const MIN_YEAR: i32 = 1;
const MAX_YEAR: i32 = 9999;
const NANOS_PER_SECOND: u32 = 1_000_000_000;
const NAIVE_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.f";

#[derive(Clone, Debug)]
pub struct SsoIdentityAssertion {
    pub id_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub issuer: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error)]
pub enum AssertionError {
    #[error("the stored SSO identity assertion payload does not hold a token and optional claims")]
    Payload(#[source] serde_json::Error),
    #[error(
        "the stored SSO identity assertion expires_at {text:?} is not an ISO 8601 date and time"
    )]
    Timestamp {
        text: String,
        #[source]
        source: ParseError,
    },
    #[error("the stored SSO identity assertion expires_at {text:?} is outside the years 1 to 9999")]
    TimestampRange { text: String },
    #[error("the SSO identity assertion could not be written as JSON")]
    Encode(#[source] serde_json::Error),
}

#[must_use]
pub fn assertion_from_sso_login(
    id_token: Option<&TypedValue>,
    refresh_token: Option<&TypedValue>,
) -> Option<SsoIdentityAssertion> {
    let raw_id_token = token_text(id_token)?;
    let raw_refresh_token = token_text(refresh_token);
    let decoded = decoded_assertion(raw_id_token, raw_refresh_token);
    if decoded.is_none() {
        tracing::warn!("{UNDECODABLE_WARNING}");
    }
    decoded
}

#[must_use]
pub fn assertion_expired(assertion: &SsoIdentityAssertion, now: DateTime<Utc>) -> bool {
    assertion
        .expires_at
        .is_some_and(|expires_at| expires_at <= now)
}

#[must_use = "the stored payload text is the reason for the call"]
pub fn encode_payload(assertion: &SsoIdentityAssertion) -> Result<String, AssertionError> {
    let id_token = assertion.id_token.expose_secret().to_owned();
    let refresh_token = assertion
        .refresh_token
        .as_ref()
        .map(ExposeSecret::expose_secret)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let issuer = assertion
        .issuer
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let expires_at = assertion.expires_at.map(|moment| moment.to_rfc3339());
    let payload = StoredPayload {
        id_token,
        refresh_token,
        issuer,
        expires_at,
    };
    serde_json::to_string(&payload).map_err(AssertionError::Encode)
}

#[must_use = "the decoded assertion is the reason for the call"]
pub fn decode_payload(text: &str) -> Result<SsoIdentityAssertion, AssertionError> {
    let payload: StoredPayload = serde_json::from_str(text).map_err(AssertionError::Payload)?;
    let expires_at = payload
        .expires_at
        .as_deref()
        .map(expires_at_text)
        .transpose()?;
    Ok(SsoIdentityAssertion {
        id_token: SecretString::from(payload.id_token),
        refresh_token: payload
            .refresh_token
            .filter(|value| !value.is_empty())
            .map(SecretString::from),
        issuer: payload.issuer,
        expires_at,
    })
}

fn token_text(field: Option<&TypedValue>) -> Option<&str> {
    field
        .and_then(|value| value.json.as_str())
        .filter(|text| !text.is_empty())
}

fn decoded_assertion(id_token: &str, refresh_token: Option<&str>) -> Option<SsoIdentityAssertion> {
    let claims = unverified_claims(id_token)?;
    let fields = claims.json.as_object()?;
    let expires_at = match fields.get("exp") {
        None => None,
        Some(exp) if exp.is_null() => None,
        Some(exp) => Some(expiration(exp.as_f64()?)?),
    };
    let issuer = match fields.get("iss") {
        None => None,
        Some(iss) if iss.is_null() => None,
        Some(iss) => Some(iss.as_str()?.to_owned()),
    };
    Some(SsoIdentityAssertion {
        id_token: SecretString::from(id_token.to_owned()),
        refresh_token: refresh_token.map(|token| SecretString::from(token.to_owned())),
        issuer,
        expires_at,
    })
}

fn expiration(seconds: f64) -> Option<DateTime<Utc>> {
    let negative = seconds.is_sign_negative();
    let duration = Duration::try_from_secs_f64(seconds.abs()).ok()?;
    let whole = i64::try_from(duration.as_secs()).ok()?;
    let fraction = duration.subsec_nanos();
    let (secs, nanos) = match (negative, fraction) {
        (true, 0) => (whole.checked_neg()?, 0),
        (true, nanos) => (
            whole.checked_neg()?.checked_sub(1)?,
            NANOS_PER_SECOND.checked_sub(nanos)?,
        ),
        (false, nanos) => (whole, nanos),
    };
    let moment = DateTime::from_timestamp(secs, nanos)?;
    (MIN_YEAR..=MAX_YEAR)
        .contains(&moment.year())
        .then_some(moment)
}

fn expires_at_text(text: &str) -> Result<DateTime<Utc>, AssertionError> {
    let moment = match DateTime::parse_from_rfc3339(text) {
        Ok(fixed) => fixed.with_timezone(&Utc),
        Err(_rfc3339) => NaiveDateTime::parse_from_str(text, NAIVE_FORMAT)
            .map(NaiveDateTime::and_utc)
            .map_err(|source| AssertionError::Timestamp {
                text: text.to_owned(),
                source,
            })?,
    };
    if !(MIN_YEAR..=MAX_YEAR).contains(&moment.year()) {
        return Err(AssertionError::TimestampRange {
            text: text.to_owned(),
        });
    }
    Ok(moment)
}

#[derive(Deserialize, Serialize)]
struct StoredPayload {
    id_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
}
