mod tests;

use aws_lc_rs::hmac;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_models::controls::json_value::JsonValue;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use thiserror::Error;

// The untrusted ``token`` cookie bytes, structurally decoded. The signature and the clock comparisons are the
// caller's trust decision: PyJWT receives the master key and its own clock, this decoder receives neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCookie {
    pub claims: SessionClaims,
    signing_input: String,
    signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionClaims {
    pub exp: i64,
    pub iat: Option<i64>,
    pub nbf: Option<i64>,
    pub user_id: ClaimText,
    pub login_method: ClaimText,
    pub session_key: ClaimText,
    pub token_type: ClaimText,
}

// A claim the identity policy inspects keeps the difference between an absent claim and a non-text one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ClaimText {
    #[default]
    Absent,
    Text(String),
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeError {
    #[error("the token is not UTF-8 text")]
    NotText,
    #[error("the token is not a compact JWT")]
    NotCompact,
    #[error("the JWT header is not canonical base64url JSON")]
    Header,
    #[error("the JWT header does not name the HS256 algorithm")]
    Algorithm,
    #[error("the JWT header key id is not a string")]
    KeyId,
    #[error("the JWT header critical extensions are not supported")]
    Critical,
    #[error("a detached b64 payload is never accepted from a session cookie")]
    DetachedPayload,
    #[error("the JWT payload is not canonical base64url JSON")]
    Payload,
    #[error("the JWT signature segment is not canonical base64url")]
    Signature,
    #[error("the JWT payload is missing the exp claim")]
    MissingExpiry,
    #[error("the JWT exp claim is not an integer")]
    Expiry,
    #[error("the JWT iat claim is not an integer")]
    IssuedAt,
    #[error("the JWT nbf claim is not an integer")]
    NotBefore,
    #[error("the JWT carries an audience but none was requested")]
    Audience,
    #[error("the JWT sub claim is not a string")]
    Subject,
    #[error("the JWT jti claim is not a string")]
    TokenId,
}

pub fn decode_session_cookie(bytes: &[u8]) -> Result<SessionCookie, DecodeError> {
    let compact = std::str::from_utf8(bytes).map_err(|_source| DecodeError::NotText)?;
    let (signing_input, signature_segment) =
        compact.rsplit_once('.').ok_or(DecodeError::NotCompact)?;
    let (header_segment, payload_segment) = signing_input
        .split_once('.')
        .ok_or(DecodeError::NotCompact)?;
    let header = segment(header_segment.as_bytes()).ok_or(DecodeError::Header)?;
    let header: JsonValue =
        serde_json::from_slice(&header).map_err(|_source| DecodeError::Header)?;
    let JsonValue::Object(header) = header else {
        return Err(DecodeError::Header);
    };
    validate_header(&header)?;
    let signature = segment(signature_segment.as_bytes()).ok_or(DecodeError::Signature)?;
    let payload = segment(payload_segment.as_bytes()).ok_or(DecodeError::Payload)?;
    let payload: JsonValue =
        serde_json::from_slice(&payload).map_err(|_source| DecodeError::Payload)?;
    let JsonValue::Object(payload) = payload else {
        return Err(DecodeError::Payload);
    };
    Ok(SessionCookie {
        claims: claims(&payload)?,
        signing_input: signing_input.to_owned(),
        signature,
    })
}

impl SessionCookie {
    // PyJWT's HS256 verification: the master key signs the header.payload bytes the cookie carried.
    #[must_use]
    pub fn signature_matches(&self, master_key: &str) -> bool {
        hmac::verify(
            &hmac::Key::new(hmac::HMAC_SHA256, master_key.as_bytes()),
            self.signing_input.as_bytes(),
            &self.signature,
        )
        .is_ok()
    }

    // PyJWT's default iat, nbf and exp checks with no leeway, at the clock the caller supplies.
    #[must_use]
    pub fn claims_are_current(&self, now: DateTime<Utc>) -> bool {
        let timestamp = now.timestamp();
        if self
            .claims
            .iat
            .is_some_and(|issued_at| issued_at > timestamp)
        {
            return false;
        }
        if self
            .claims
            .nbf
            .is_some_and(|not_before| not_before > timestamp)
        {
            return false;
        }
        self.claims.exp > timestamp
    }
}

// PyJWT's segment rules: at most two trailing '=', a padded segment of whole quads, and a canonical
// re-encoding. The engine rejects a stripped length that is one past a quad, which PyJWT checks itself.
fn segment(segment: &[u8]) -> Option<Vec<u8>> {
    let padding = segment
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count();
    if padding > 2 || (padding > 0 && !segment.len().is_multiple_of(4)) {
        return None;
    }
    let symbols = segment.get(..segment.len().saturating_sub(padding))?;
    let decoded = URL_SAFE_NO_PAD.decode(symbols).ok()?;
    (URL_SAFE_NO_PAD.encode(&decoded).as_bytes() == symbols).then_some(decoded)
}

fn validate_header(header: &IndexMap<String, JsonValue>) -> Result<(), DecodeError> {
    if !matches!(header.get("alg"), Some(JsonValue::Text(algorithm)) if algorithm == "HS256") {
        return Err(DecodeError::Algorithm);
    }
    if let Some(key_id) = header.get("kid")
        && !matches!(key_id, JsonValue::Text(_))
    {
        return Err(DecodeError::KeyId);
    }
    if let Some(critical) = header.get("crit") {
        let JsonValue::Array(extensions) = critical else {
            return Err(DecodeError::Critical);
        };
        if extensions.is_empty() {
            return Err(DecodeError::Critical);
        }
        for extension in extensions {
            let JsonValue::Text(name) = extension else {
                return Err(DecodeError::Critical);
            };
            if name != "b64" || !header.contains_key(name) {
                return Err(DecodeError::Critical);
            }
        }
    }
    if matches!(header.get("b64"), Some(JsonValue::Bool(false))) {
        return Err(DecodeError::DetachedPayload);
    }
    Ok(())
}

fn claims(payload: &IndexMap<String, JsonValue>) -> Result<SessionClaims, DecodeError> {
    let Some(expiry) = payload.get("exp") else {
        return Err(DecodeError::MissingExpiry);
    };
    let exp = integer(expiry).ok_or(DecodeError::Expiry)?;
    let iat = payload
        .get("iat")
        .map(|value| integer(value).ok_or(DecodeError::IssuedAt))
        .transpose()?;
    let nbf = payload
        .get("nbf")
        .map(|value| integer(value).ok_or(DecodeError::NotBefore))
        .transpose()?;
    validate_audience(payload.get("aud"))?;
    if let Some(subject) = payload.get("sub")
        && !matches!(subject, JsonValue::Text(_))
    {
        return Err(DecodeError::Subject);
    }
    if let Some(token_id) = payload.get("jti")
        && !matches!(token_id, JsonValue::Text(_))
    {
        return Err(DecodeError::TokenId);
    }
    Ok(SessionClaims {
        exp,
        iat,
        nbf,
        user_id: claim_text(payload, "user_id"),
        login_method: claim_text(payload, "login_method"),
        session_key: claim_text(payload, "key"),
        token_type: claim_text(payload, "type"),
    })
}

fn integer(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Number(number) => time_value(number),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Text(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}

// PyJWT compares Python's unbounded ints, so an integer literal outside i64 keeps its side of every
// comparison when clamped; a float literal stays a wrong-typed claim.
fn time_value(number: &serde_json::Number) -> Option<i64> {
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    let literal = number.to_string();
    if literal
        .bytes()
        .any(|byte| matches!(byte, b'.' | b'e' | b'E'))
    {
        return None;
    }
    let Ok(parsed) = literal.parse::<i128>() else {
        return Some(if literal.starts_with('-') {
            i64::MIN
        } else {
            i64::MAX
        });
    };
    if let Ok(value) = i64::try_from(parsed) {
        return Some(value);
    }
    Some(if parsed.is_negative() {
        i64::MIN
    } else {
        i64::MAX
    })
}

// LiteLLM requests no audience, so PyJWT refuses any audience the token carries, empty ones aside.
fn validate_audience(audience: Option<&JsonValue>) -> Result<(), DecodeError> {
    match audience {
        None | Some(JsonValue::Null) => Ok(()),
        Some(JsonValue::Text(value)) if value.is_empty() => Ok(()),
        Some(JsonValue::Array(entries)) if entries.is_empty() => Ok(()),
        Some(_) => Err(DecodeError::Audience),
    }
}

fn claim_text(payload: &IndexMap<String, JsonValue>, name: &str) -> ClaimText {
    match payload.get(name) {
        Some(JsonValue::Text(text)) => ClaimText::Text(text.clone()),
        Some(JsonValue::Null) | None => ClaimText::Absent,
        Some(_) => ClaimText::Other,
    }
}
