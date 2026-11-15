use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;

use super::error::CompatibleChatError;

// what an optional_params value is read as, and the field type its error names
pub(crate) trait OptionValue: Sized {
    const EXPECTED: &'static str;

    fn pick(value: Json) -> Option<Self>;
}

impl OptionValue for bool {
    const EXPECTED: &'static str = "a boolean";

    fn pick(value: Json) -> Option<Self> {
        match value {
            Json::Bool(value) => Some(value),
            _ => None,
        }
    }
}

impl OptionValue for String {
    const EXPECTED: &'static str = "a string";

    fn pick(value: Json) -> Option<Self> {
        match value {
            Json::Text(value) => Some(value),
            _ => None,
        }
    }
}

impl OptionValue for IndexMap<String, Json> {
    const EXPECTED: &'static str = "a JSON object";

    fn pick(value: Json) -> Option<Self> {
        match value {
            Json::Object(value) => Some(value),
            _ => None,
        }
    }
}

// optional_params.pop(name, None): absent and null leave the default, and a value of the wrong type names the field
pub(crate) fn take<T: OptionValue>(
    options: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<T>, CompatibleChatError> {
    match options.shift_remove(field) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => T::pick(value)
            .map(Some)
            .ok_or(CompatibleChatError::OptionType {
                field,
                expected: T::EXPECTED,
            }),
    }
}

pub(crate) fn headers_json(headers: &Headers) -> IndexMap<String, Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect()
}
