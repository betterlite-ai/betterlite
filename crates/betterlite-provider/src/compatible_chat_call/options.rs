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
            Json::Bool(flag) => Some(flag),
            Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

impl OptionValue for String {
    const EXPECTED: &'static str = "a string";

    fn pick(value: Json) -> Option<Self> {
        match value {
            Json::Text(text) => Some(text),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

impl OptionValue for IndexMap<String, Json> {
    const EXPECTED: &'static str = "a JSON object";

    fn pick(value: Json) -> Option<Self> {
        match value {
            Json::Object(object) => Some(object),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        }
    }
}

// optional_params.pop(name, None): absent and null leave the default, and a value of the wrong type names the field
pub(crate) fn take<T>(
    options: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<T>, CompatibleChatError>
where
    T: OptionValue,
{
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
