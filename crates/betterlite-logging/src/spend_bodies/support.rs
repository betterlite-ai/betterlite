use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, MAX_DEPTH_MARKER};
use crate::spend_sanitize::request_body::sanitize_request_body;
use crate::spend_sanitize::truncate::{TRUNCATED_PAYLOAD_FIELD, TRUNCATION_DB_SAFEGUARD_NOTE};
use crate::spend_sanitize::value::SpendValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
pub type Fields = IndexMap<String, Json>;
pub(super) const fn as_object(value: &Json) -> Option<&Fields> {
    if let Json::Object(fields) = value {
        return Some(fields);
    }
    None
}
pub(super) const fn as_object_mut(value: &mut Json) -> Option<&mut Fields> {
    if let Json::Object(fields) = value {
        return Some(fields);
    }
    None
}
// B1713 owns the projection, so a mapping becomes the sanitizer's tree with fresh identities
pub(super) fn sanitize_fields(fields: &Fields, max_string_length: i64) -> Json {
    let SpendValue::Mapping(mapping) = SpendValue::from(&Json::Object(fields.clone())) else {
        return Json::Object(Fields::new());
    };
    sanitize_request_body(&mapping, max_string_length)
}
// LiteLLM asks `if value:` everywhere a policy flag or an overlay is read
pub(super) fn counts_as_present(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number
            .as_f64()
            .is_none_or(|float| float.to_bits() & 0x7fff_ffff_ffff_ffff != 0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
pub(super) fn log_truncation(text: &str, what: &str) {
    if text.contains(TRUNCATED_PAYLOAD_FIELD) {
        tracing::info!(
            "Spend Log: {what} was truncated before storing in DB. {}",
            TRUNCATION_DB_SAFEGUARD_NOTE
        );
    }
}
// LiteLLM's safe_json_structure: strip NULs and replace anything past the depth bound
pub(super) fn safe_structure(value: &Json, depth: usize) -> Json {
    if depth > DEFAULT_MAX_RECURSE_DEPTH {
        return Json::Text(MAX_DEPTH_MARKER.to_owned());
    }
    let nested = depth.saturating_add(1);
    match value {
        Json::Text(text) => Json::Text(text.replace('\0', "")),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| safe_structure(item, nested))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| (key.replace('\0', ""), safe_structure(item, nested)))
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) => value.clone(),
    }
}
