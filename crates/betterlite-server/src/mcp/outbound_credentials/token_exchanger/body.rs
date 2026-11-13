use std::collections::BTreeMap;

use chrono::{DateTime, TimeDelta, Utc};

use super::super::OAuthToken;
use super::super::v2_store::CredentialValue;
use super::NON_ACCESS_ISSUED_TOKEN_TYPES;

// Python's int() truncates a float toward zero; a value outside i64 has no bounded equivalent.
fn truncate(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    value.trunc().to_string().parse::<i64>().ok()
}

pub(super) fn parse_expires_in(value: Option<&CredentialValue>) -> Option<i64> {
    match value? {
        CredentialValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().and_then(truncate)),
        CredentialValue::Text(text) => text.trim().parse::<f64>().ok().and_then(truncate),
        CredentialValue::Bool(_)
        | CredentialValue::Null
        | CredentialValue::Array(_)
        | CredentialValue::Object(_) => None,
    }
}

fn expires_at(now: DateTime<Utc>, seconds: i64) -> DateTime<Utc> {
    TimeDelta::try_seconds(seconds)
        .and_then(|delta| now.checked_add_signed(delta))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

pub(super) fn token_from_body(
    body: &BTreeMap<String, CredentialValue>,
    now: DateTime<Utc>,
) -> Option<OAuthToken> {
    let Some(CredentialValue::Text(access_token)) = body.get("access_token") else {
        return None;
    };
    if access_token.is_empty() {
        return None;
    }
    // token_type is forwarded as Bearer, so a present non-Bearer type fails closed rather than minting one.
    if let Some(CredentialValue::Text(token_type)) = body.get("token_type")
        && !token_type.trim().eq_ignore_ascii_case("bearer")
    {
        tracing::warn!(%token_type, "MCP token exchange returned an unusable token_type");
        return None;
    }
    // A malformed STS could label a refresh, id or saml token as Bearer, so the issued type decides too.
    if let Some(CredentialValue::Text(issued)) = body.get("issued_token_type")
        && NON_ACCESS_ISSUED_TOKEN_TYPES.contains(&issued.as_str())
    {
        return None;
    }
    Some(OAuthToken {
        access_token: access_token.clone(),
        expires_at: parse_expires_in(body.get("expires_in"))
            .map(|seconds| expires_at(now, seconds)),
        refresh_token: None,
        scopes: Vec::new(),
        identity_binding_proof: None,
    })
}
