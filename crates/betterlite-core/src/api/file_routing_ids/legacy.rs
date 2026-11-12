use std::collections::BTreeMap;

use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use crate::api::managed_resource_ids::is_base64_encoded_unified_id;

use super::error::FileRoutingIdsError;
use super::{EXECUTED_BATCH_ID_PREFIX, MAX_FILE_LIST_LIMIT};

const MANAGED_FILE_ID_PREFIX: &str = "litellm_proxy";
const TARGET_MODELS_FIELD: &str = "target_model_names,";
const MODEL_ID_FIELD: &str = "model_id:";
const BATCH_ID_FIELD: &str = "llm_batch_id:";
const GENERIC_RESPONSE_ID_FIELD: &str = "generic_response_id:";
const BATCH_ID_MARKER: &str = "llm_batch_id";
const BATCH_ID_TERMINATORS: [char; 2] = [';', ','];

// str.strip() also removes U+001C to U+001F, which char::is_whitespace keeps
pub(super) fn is_strip_whitespace(candidate: char) -> bool {
    if candidate.is_whitespace() {
        true
    } else {
        ('\u{1c}'..='\u{1f}').contains(&candidate)
    }
}

pub fn validate_file_list_limit(limit: Option<i64>) -> Result<(), FileRoutingIdsError> {
    let Some(requested) = limit else {
        return Ok(());
    };
    let (message, openai_code) = match requested {
        1..=MAX_FILE_LIST_LIMIT => return Ok(()),
        i64::MIN..1 => (
            format!(
                "Invalid 'limit': integer below minimum value. Expected a value >= 1, but got {requested} instead."
            ),
            "integer_below_min_value",
        ),
        _ => (
            format!(
                "Invalid 'limit': integer above maximum value. Expected a value <= {MAX_FILE_LIST_LIMIT}, but got {requested} instead."
            ),
            "integer_above_max_value",
        ),
    };
    Err(FileRoutingIdsError::InvalidLimit(Box::new(
        ProxyException::new(ProxyExceptionInit {
            message,
            error_type: "invalid_request_error".to_owned(),
            param: Some("limit".to_owned()),
            code: ExceptionCode::Status(400),
            openai_code: Some(openai_code.to_owned()),
            headers: BTreeMap::new(),
            provider_specific_fields: BTreeMap::new(),
        }),
    )))
}

#[must_use]
pub fn decode_legacy_unified_id(resource_id: &str) -> Option<String> {
    if let Some(decoded) = is_base64_encoded_unified_id(resource_id, MANAGED_FILE_ID_PREFIX) {
        return Some(decoded);
    }
    None
}

#[must_use]
pub fn convert_b64_uid_to_unified_uid(b64_uid: &str) -> String {
    if let Some(decoded) = decode_legacy_unified_id(b64_uid) {
        return decoded;
    }
    b64_uid.to_owned()
}

#[must_use]
pub fn target_model_names(text: &str) -> Vec<String> {
    text.split(';')
        .find_map(|field| {
            field
                .split_once(TARGET_MODELS_FIELD)
                .map(|(_, names)| names)
                .filter(|names| !names.is_empty())
        })
        .map(|names| {
            names
                .split(',')
                .map(|name| name.trim_matches(is_strip_whitespace).to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[must_use]
pub fn resolve_output_model(
    unified_input_file_id: Option<&str>,
    fallback_model_name: Option<&str>,
) -> Option<String> {
    let source = match unified_input_file_id {
        Some(file_id) => convert_b64_uid_to_unified_uid(file_id),
        None => return fallback_model_name.map(str::to_owned),
    };
    let names = target_model_names(&source);
    if names.is_empty() {
        return fallback_model_name.map(str::to_owned);
    }
    Some(names.join(","))
}

#[must_use]
pub fn unified_batch_model_id(file_id: &str) -> Option<String> {
    file_id
        .split(MODEL_ID_FIELD)
        .nth(1)
        .and_then(|after| after.split(';').next())
        .map(str::to_owned)
}

pub fn unified_batch_id(file_id: &str) -> Result<String, FileRoutingIdsError> {
    let rest = if file_id.contains(BATCH_ID_MARKER) {
        file_id.split_once(BATCH_ID_FIELD)
    } else {
        file_id.split_once(GENERIC_RESPONSE_ID_FIELD)
    };
    let (_, remainder) = rest.ok_or(FileRoutingIdsError::MissingBatchMarker)?;
    let batch_id = remainder
        .chars()
        .take_while(|character| !BATCH_ID_TERMINATORS.contains(character))
        .collect::<String>();
    Ok(batch_id)
}

#[must_use]
pub fn is_litellm_executed_batch(decoded_batch_id: &str) -> bool {
    let Some((_, batch_id)) = decoded_batch_id.split_once(BATCH_ID_FIELD) else {
        return false;
    };
    batch_id.starts_with(EXECUTED_BATCH_ID_PREFIX)
}
