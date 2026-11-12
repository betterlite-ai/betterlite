use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_redis::cache_primitives::decode::{CachedValue, DecodeError, decode_cached_value};

use super::payload::{ReadOperation, log_decode_miss};
use crate::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};

pub const MAX_PAYLOAD_BYTES: usize = 1_048_576;
pub const MAX_PAYLOAD_DEPTH: usize = 64;

const AUTH_MODEL: &str = "UserAPIKeyAuth";

// Redis hands back raw bytes, so the shared json decoder owns the shape and the wrapper only sees the miss
pub fn decode_cached_auth(
    bytes: &[u8],
    operation: ReadOperation,
) -> Result<Option<UserAPIKeyAuth>, AuthError> {
    if bytes.len() > MAX_PAYLOAD_BYTES {
        tracing::error!(
            operation = operation.label(),
            bytes = bytes.len(),
            limit = MAX_PAYLOAD_BYTES,
            "cached auth payload exceeds the cache payload limits"
        );
        return Ok(None);
    }
    let decoded = match decode_cached_value(Some(bytes)) {
        Ok(decoded) => decoded,
        Err(error) => {
            tracing::error!(
                operation = operation.label(),
                kind = decode_error_kind(&error),
                "cached auth payload is not decodable"
            );
            return Ok(None);
        }
    };
    let Some(value) = decoded else {
        return Ok(None);
    };
    if depth(&value) > MAX_PAYLOAD_DEPTH {
        tracing::error!(
            operation = operation.label(),
            limit = MAX_PAYLOAD_DEPTH,
            "cached auth payload exceeds the cache payload depth limit"
        );
        return Ok(None);
    }
    let CachedValue::Object(entries) = value else {
        log_decode_miss(operation, AUTH_MODEL);
        return Ok(None);
    };
    let fields = entries
        .into_iter()
        .map(|(key, entry)| (key, json_data(entry)))
        .collect();
    construct_auth(fields, operation)
}

pub fn decode_auth_payload(
    payload: &Json,
    operation: ReadOperation,
) -> Result<Option<UserAPIKeyAuth>, AuthError> {
    if !matches!(payload, Json::Object(_)) {
        log_decode_miss(operation, AUTH_MODEL);
        return Ok(None);
    }
    let wire = serde_json::to_value(payload).map_err(AuthError::Json)?;
    let fields =
        serde_json::from_value::<BTreeMap<String, JsonData>>(wire).map_err(AuthError::Json)?;
    construct_auth(fields, operation)
}

fn construct_auth(
    fields: BTreeMap<String, JsonData>,
    operation: ReadOperation,
) -> Result<Option<UserAPIKeyAuth>, AuthError> {
    match UserAPIKeyAuth::construct(AuthInput::Dict(fields)) {
        Ok(auth) => Ok(Some(auth)),
        Err(error @ AuthError::NonStringApiKey) => Err(error),
        Err(error) => {
            tracing::warn!(
                model = AUTH_MODEL,
                kind = auth_error_kind(&error),
                "CacheCodec.deserialize: validation failed"
            );
            log_decode_miss(operation, AUTH_MODEL);
            Ok(None)
        }
    }
}

// iterative so a hostile payload bounded only by the byte limit cannot overflow the stack
pub(crate) fn depth(value: &CachedValue) -> usize {
    let mut maximum = 0;
    let mut pending = vec![(value, 1_usize)];
    while let Some((current, level)) = pending.pop() {
        match current {
            CachedValue::Array(items) => {
                maximum = maximum.max(level);
                pending.extend(items.iter().map(|item| (item, level.saturating_add(1))));
            }
            CachedValue::Object(entries) => {
                maximum = maximum.max(level);
                pending.extend(
                    entries
                        .values()
                        .map(|entry| (entry, level.saturating_add(1))),
                );
            }
            CachedValue::Null
            | CachedValue::Boolean(_)
            | CachedValue::Number(_)
            | CachedValue::Text(_) => {}
        }
    }
    maximum
}

fn json_data(value: CachedValue) -> JsonData {
    match value {
        CachedValue::Null => JsonData::Null,
        CachedValue::Boolean(flag) => JsonData::Bool(flag),
        CachedValue::Number(number) => JsonData::Number(number),
        CachedValue::Text(text) => JsonData::String(text),
        CachedValue::Array(items) => JsonData::Array(items.into_iter().map(json_data).collect()),
        CachedValue::Object(entries) => JsonData::Object(
            entries
                .into_iter()
                .map(|(key, entry)| (key, json_data(entry)))
                .collect(),
        ),
    }
}

const fn decode_error_kind(error: &DecodeError) -> &'static str {
    match error {
        DecodeError::Utf8 { .. } => "utf8",
        DecodeError::Count { .. } => "count",
        DecodeError::LiteralEval { .. } => "literal_eval",
        DecodeError::UnsupportedLiteral => "unsupported_literal",
        DecodeError::IntegerOutOfRange => "integer_out_of_range",
        DecodeError::NonFiniteFloat => "non_finite_float",
        DecodeError::NonStringKey => "non_string_key",
    }
}

const fn auth_error_kind(error: &AuthError) -> &'static str {
    match error {
        AuthError::NonStringApiKey => "non_string_api_key",
        AuthError::View(_) => "view",
        AuthError::Json(_) => "json",
    }
}
