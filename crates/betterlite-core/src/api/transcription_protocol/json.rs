use betterlite_models::decisions::Json;
use serde_json::Number;

use super::JsonObject;
// ast-grep-ignore: no-long-names
use super::RealtimeTranscriptionProtocolError as ProtocolError;

pub(super) const fn present(value: Option<&Json>) -> Option<&Json> {
    match value {
        Some(Json::Null) => None,
        other => other,
    }
}

pub(super) trait FromField<'json>: Sized {
    fn from_field(value: &'json Json) -> Option<Self>;

    fn field_error(name: &'static str) -> ProtocolError;
}

impl<'json> FromField<'json> for &'json JsonObject {
    fn from_field(value: &'json Json) -> Option<Self> {
        match value {
            Json::Object(fields) => Some(fields),
            _ => None,
        }
    }

    fn field_error(name: &'static str) -> ProtocolError {
        ProtocolError::FieldNotObject { name }
    }
}

impl<'json> FromField<'json> for &'json str {
    fn from_field(value: &'json Json) -> Option<Self> {
        match value {
            Json::Text(text) => Some(text),
            _ => None,
        }
    }

    fn field_error(name: &'static str) -> ProtocolError {
        ProtocolError::FieldNotString { name }
    }
}

impl<'json> FromField<'json> for Number {
    fn from_field(value: &'json Json) -> Option<Self> {
        match value {
            Json::Number(number) if is_integer(number) => Some(number.clone()),
            _ => None,
        }
    }

    fn field_error(name: &'static str) -> ProtocolError {
        ProtocolError::FieldNotInteger { name }
    }
}

pub(super) fn field<'json, T, E>(
    value: Option<&'json Json>,
    name: &'static str,
) -> Result<Option<T>, E>
where
    T: FromField<'json>,
    E: From<ProtocolError>,
{
    match present(value) {
        None => Ok(None),
        Some(value) => T::from_field(value)
            .map(Some)
            .ok_or_else(|| E::from(T::field_error(name))),
    }
}

fn is_integer(number: &Number) -> bool {
    number
        .to_string()
        .chars()
        .all(|character| !matches!(character, '.' | 'e' | 'E'))
}
