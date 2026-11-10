use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, MAX_DEPTH_MARKER};
use crate::python_json;
use crate::spend_sanitize::request_body::sanitize_request_body;
use crate::spend_sanitize::value::{SpendMapping, SpendValue};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;
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
// B1713 owns the generic projection, so a mapping becomes the sanitizer's tree with fresh identities
pub(super) fn spend_mapping(fields: &Fields) -> Option<SpendMapping> {
    let SpendValue::Mapping(mapping) = SpendValue::from(&Json::Object(fields.clone())) else {
        return None;
    };
    Some(mapping)
}
pub(super) fn sanitize_fields(fields: &Fields, max_string_length: i64) -> Json {
    let Some(mapping) = spend_mapping(fields) else {
        return Json::Object(Fields::new());
    };
    sanitize_request_body(&mapping, max_string_length)
}
// LiteLLM asks `if value:` everywhere a policy flag or an overlay is read
pub(super) fn counts_as_present(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number_is_nonzero(number),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
fn number_is_nonzero(number: &Number) -> bool {
    number
        .to_string()
        .parse::<f64>()
        .map_or(true, |value| (value.to_bits() & 0x7fff_ffff_ffff_ffff) != 0)
}
// Python's str() for the declared string types, with containers projected as JSON
pub(super) fn string_value(value: &Json) -> String {
    if let Json::Text(text) = value {
        return text.clone();
    }
    if matches!(value, Json::Null) {
        return "None".to_owned();
    }
    if let Json::Bool(flag) = value {
        return (if *flag { "True" } else { "False" }).to_owned();
    }
    if let Json::Number(number) = value {
        return python_json::dumps(&Json::Number(number.clone()));
    }
    python_json::dumps(value)
}
// LiteLLM's safe_json_structure: strip NULs and replace anything past the depth bound
pub(super) fn safe_structure(value: &Json, depth: usize) -> Json {
    if depth > DEFAULT_MAX_RECURSE_DEPTH {
        return Json::Text(MAX_DEPTH_MARKER.to_owned());
    }
    match value {
        Json::Text(text) => Json::Text(text.replace('\0', "")),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| safe_structure(item, depth.saturating_add(1)))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| {
                    (
                        key.replace('\0', ""),
                        safe_structure(item, depth.saturating_add(1)),
                    )
                })
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) => value.clone(),
    }
}
