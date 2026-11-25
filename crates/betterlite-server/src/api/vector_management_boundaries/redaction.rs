use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_redact::structured::SensitiveDataMasker;
use indexmap::IndexMap;

use super::error::VectorManagementError;
use super::json::decode_management_json;

pub const REDACTED_BY_LITELM: &str = "REDACTED_BY_LITELM";
pub const REDACT_PARAMS_MAX_DEPTH: usize = 10;

// _LITELLM_PARAMS_MASKER: the shared classifier plus the one extra segment litellm_params uses
const MASKER_PATTERNS: [&str; 1] = ["connection"];

// _redact_sensitive_litellm_params: credential-bearing values become the sentinel, ten parses deep at most
#[must_use]
pub fn redact_sensitive_litellm_params(params: &Json, depth: usize, limits: ApiLimits) -> Json {
    let masker = SensitiveDataMasker::with_patterns(MASKER_PATTERNS, std::iter::empty::<&str>());
    redact_at(&masker, params, depth, limits)
}

// the record's litellm_params redacted in place: a stored JSON string stays a JSON string
pub fn redact_store_params(
    record: &LiteLlmManagedVectorStore,
    limits: ApiLimits,
) -> Result<LiteLlmManagedVectorStore, VectorManagementError> {
    let Some(params) = &record.litellm_params else {
        return Ok(record.clone());
    };
    let parsed: Json = serde_json::from_value(params.clone())?;
    let mut redacted = record.clone();
    redacted.litellm_params = Some(serde_json::to_value(redact_sensitive_litellm_params(
        &parsed, 0, limits,
    ))?);
    Ok(redacted)
}

fn redact_at(masker: &SensitiveDataMasker, value: &Json, depth: usize, limits: ApiLimits) -> Json {
    if depth >= REDACT_PARAMS_MAX_DEPTH {
        return Json::Text(REDACTED_BY_LITELM.to_owned());
    }
    match value {
        Json::Null => Json::Null,
        Json::Text(text) => redact_encoded(masker, text, depth, limits),
        Json::Object(fields) => redact_object(masker, fields, depth, limits),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| redact_nested(masker, item, depth, limits))
                .collect(),
        ),
        Json::Bool(flag) => Json::Bool(*flag),
        Json::Number(number) => Json::Number(number.clone()),
    }
}

fn redact_object(
    masker: &SensitiveDataMasker,
    fields: &IndexMap<String, Json>,
    depth: usize,
    limits: ApiLimits,
) -> Json {
    let mut redacted = IndexMap::with_capacity(fields.len());
    for (key, value) in fields {
        let masked = if masker.is_sensitive_key(key, &[]) {
            Json::Text(REDACTED_BY_LITELM.to_owned())
        } else {
            redact_nested(masker, value, depth, limits)
        };
        redacted.insert(key.clone(), masked);
    }
    Json::Object(redacted)
}

// a buried object or array is one level deeper; every other value comes back as it is, never parsed
fn redact_nested(
    masker: &SensitiveDataMasker,
    value: &Json,
    depth: usize,
    limits: ApiLimits,
) -> Json {
    if matches!(value, Json::Object(_) | Json::List(_)) {
        return redact_at(masker, value, depth.saturating_add(1), limits);
    }
    value.clone()
}

// a str is json.loads, redacted one parse deeper, then json.dumps; text that does not parse is the sentinel
fn redact_encoded(
    masker: &SensitiveDataMasker,
    text: &str,
    depth: usize,
    limits: ApiLimits,
) -> Json {
    let Ok(parsed) = decode_management_json(text.as_bytes(), limits) else {
        return Json::Text(REDACTED_BY_LITELM.to_owned());
    };
    let nested = redact_at(masker, &parsed, depth.saturating_add(1), limits);
    Json::Text(dumps(&nested))
}
