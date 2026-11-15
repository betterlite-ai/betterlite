use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;

use super::error::CompatibleChatError;

// optional_params.pop(name, None): absent and null leave the default, and a value of the wrong type names the field
pub(crate) fn take_bool(
    options: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<bool>, CompatibleChatError> {
    match options.shift_remove(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(value)),
        Some(_) => Err(CompatibleChatError::OptionType {
            field,
            expected: "a boolean",
        }),
    }
}

pub(crate) fn take_text(
    options: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, CompatibleChatError> {
    match options.shift_remove(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value)),
        Some(_) => Err(CompatibleChatError::OptionType {
            field,
            expected: "a string",
        }),
    }
}

pub(crate) fn take_object(
    options: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<IndexMap<String, Json>>, CompatibleChatError> {
    match options.shift_remove(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(value)) => Ok(Some(value)),
        Some(_) => Err(CompatibleChatError::OptionType {
            field,
            expected: "a JSON object",
        }),
    }
}

pub(crate) fn headers_json(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}
