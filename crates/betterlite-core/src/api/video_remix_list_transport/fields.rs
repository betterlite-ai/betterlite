use betterlite_models::decisions::Json;
use secrecy::SecretString;

use super::error::FieldError;
use super::records::Params;

// A params value of a declared type: absent or null is no value, and a value of another type names the field
pub(super) trait FieldValue: Sized {
    const EXPECTED: &'static str;

    fn parse(value: &Json) -> Option<Self>;
}

impl FieldValue for bool {
    const EXPECTED: &'static str = "a boolean";

    fn parse(value: &Json) -> Option<Self> {
        match value {
            Json::Bool(found) => Some(*found),
            Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

impl FieldValue for String {
    const EXPECTED: &'static str = "a string";

    fn parse(value: &Json) -> Option<Self> {
        match value {
            Json::Text(text) => Some(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

impl FieldValue for SecretString {
    const EXPECTED: &'static str = "a string";

    fn parse(value: &Json) -> Option<Self> {
        match value {
            Json::Text(text) => Some(SecretString::from(text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}

// A param read where the handler reads it: the field is only touched when that call asks for it
pub(super) fn read<T>(params: &Params, name: &'static str) -> Result<Option<T>, FieldError>
where
    T: FieldValue,
{
    let Some(value) = params.get(name).filter(|given| **given != Json::Null) else {
        return Ok(None);
    };
    match T::parse(value) {
        Some(parsed) => Ok(Some(parsed)),
        None => Err(FieldError::Type {
            field: name,
            expected: T::EXPECTED,
        }),
    }
}
