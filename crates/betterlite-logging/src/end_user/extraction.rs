use betterlite_config::sections::request_headers::{Settings, UserHeaderMapping, UserHeaderRole};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use http::HeaderMap;

use crate::private_json::safe_json::safe_json_loads;
use crate::python_json;

mod tests;

const USER_FIELD: &str = "user";
const LITELLM_METADATA_FIELD: &str = "litellm_metadata";
const METADATA_FIELD: &str = "metadata";
const METADATA_USER_FIELD: &str = "user_id";
const SAFETY_IDENTIFIER_FIELD: &str = "safety_identifier";

// STANDARD_CUSTOMER_ID_HEADERS in LiteLLM's order: the customer id outranks every configured source
const STANDARD_CUSTOMER_ID_HEADERS: [&str; 2] = ["x-litellm-customer-id", "x-litellm-end-user-id"];

// the two settings this lookup reads: the header mappings and whether ids are validated against the database
#[derive(Clone, Copy, Debug)]
pub struct IdSettings<'config> {
    pub request_headers: &'config Settings,
    pub validate_end_user_id_in_db: bool,
}

// get_customer_user_header_from_mapping: the customer mappings' header names, lowercased and in list order
#[must_use]
pub fn customer_header_names(mappings: &[UserHeaderMapping]) -> Vec<String> {
    mappings
        .iter()
        .filter(|mapping| {
            mapping.litellm_user_role == UserHeaderRole::Customer && !mapping.header_name.is_empty()
        })
        .map(|mapping| mapping.header_name.to_lowercase())
        .collect()
}

#[must_use]
pub fn request_end_user_id(
    body: &JsonObject,
    headers: Option<&HeaderMap>,
    settings: &IdSettings<'_>,
) -> Option<String> {
    if let Some(request_headers) = headers
        && let Some(end_user_id) = header_end_user_id(request_headers, settings)
    {
        return Some(end_user_id);
    }
    body_end_user_id(body, settings.validate_end_user_id_in_db)
}

fn header_end_user_id(headers: &HeaderMap, settings: &IdSettings<'_>) -> Option<String> {
    let validate = settings.validate_end_user_id_in_db;
    standard_header_id(headers, validate)
        .or_else(|| configured_header_id(headers, settings.request_headers, validate))
}

// _get_customer_id_from_standard_headers: the two standard headers always count, whatever is configured
fn standard_header_id(headers: &HeaderMap, validate: bool) -> Option<String> {
    for standard in STANDARD_CUSTOMER_ID_HEADERS {
        if let Some(end_user_id) = header_id(headers, standard, validate) {
            return Some(end_user_id);
        }
    }
    None
}

fn configured_header_id(
    headers: &HeaderMap,
    settings: &Settings,
    validate: bool,
) -> Option<String> {
    let customer_headers = customer_header_names(&settings.user_header_mappings);
    if !customer_headers.is_empty() {
        return customer_headers
            .iter()
            .find_map(|name| header_id(headers, name, validate));
    }
    let name = settings.user_header_name.as_deref()?;
    if strip_space(name).is_empty() {
        return None;
    }
    header_id(headers, name, validate)
}

// Starlette decodes header bytes as latin-1, so every byte is a character and the value is never refused
fn header_id(headers: &HeaderMap, name: &str, validate: bool) -> Option<String> {
    let value = headers.get(name)?;
    let text: String = value
        .as_bytes()
        .iter()
        .map(|byte| char::from(*byte))
        .collect();
    coerce_user_id(&Json::Text(text), validate)
}

fn body_end_user_id(body: &JsonObject, validate: bool) -> Option<String> {
    if let Some(end_user_id) = body
        .get(USER_FIELD)
        .and_then(|value| coerce_user_id(value, validate))
    {
        return Some(end_user_id);
    }
    if let Some(end_user_id) = metadata_id(body, LITELLM_METADATA_FIELD, USER_FIELD, validate) {
        return Some(end_user_id);
    }
    if let Some(end_user_id) = metadata_id(body, METADATA_FIELD, METADATA_USER_FIELD, validate) {
        return Some(end_user_id);
    }
    body.get(SAFETY_IDENTIFIER_FIELD)
        .and_then(|value| coerce_user_id(value, validate))
}

// the nested _as_dict: an object counts as is, a string only when it decodes to an object, and null counts as empty
fn metadata_id(body: &JsonObject, bucket: &str, field: &str, validate: bool) -> Option<String> {
    match body.get(bucket) {
        Some(Json::Object(fields)) => fields
            .get(field)
            .and_then(|value| coerce_user_id(value, validate)),
        Some(Json::Text(text)) => decoded_metadata_id(text, field, validate),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => None,
    }
}

fn decoded_metadata_id(text: &str, field: &str, validate: bool) -> Option<String> {
    let Ok(parsed) = safe_json_loads(text.as_bytes()) else {
        return None;
    };
    let Ok(Json::Object(fields)) = serde_json::from_value::<Json>(parsed) else {
        return None;
    };
    fields
        .get(field)
        .and_then(|value| coerce_user_id(value, validate))
}

// _coerce_user_id_to_str: a number keeps Python's str text, a string is trimmed, and everything else is dropped
#[must_use]
pub fn coerce_user_id(value: &Json, validate: bool) -> Option<String> {
    match value {
        Json::Number(_) => Some(python_json::dumps(value)),
        Json::Text(text) => user_id_text(text, validate),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn user_id_text(text: &str, validate: bool) -> Option<String> {
    let stripped = strip_space(text);
    if stripped.is_empty() {
        return None;
    }
    // a JSON object or list is only refused once the operator opted into end-user validation
    if validate && stripped.starts_with(['{', '[']) {
        let parsed = safe_json_loads(stripped.as_bytes());
        if matches!(parsed, Ok(value) if value.is_object() || value.is_array()) {
            return None;
        }
    }
    Some(stripped.to_owned())
}

// Python's str.strip also drops the U+001C to U+001F separators, which char::is_whitespace leaves in place
fn strip_space(text: &str) -> &str {
    text.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    })
}
