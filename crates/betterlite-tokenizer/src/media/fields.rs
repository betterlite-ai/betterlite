use betterlite_models::decisions::Json;

use super::error::MediaError;
use super::wire::Object;

pub(crate) type FieldResult<T> = Result<T, MediaError>;

pub(crate) fn text(fields: &Object, name: &'static str) -> FieldResult<Option<String>> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(MediaError::Field { field: name }),
    }
}

pub(crate) fn object<'fields>(
    fields: &'fields Object,
    name: &'static str,
) -> FieldResult<Option<&'fields Object>> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(object)) => Ok(Some(object)),
        Some(_) => Err(MediaError::Field { field: name }),
    }
}

pub(crate) fn required_text(fields: &Object, name: &'static str) -> FieldResult<String> {
    let Some(value) = text(fields, name)? else {
        return Err(MediaError::Field { field: name });
    };
    Ok(value)
}

pub(crate) fn require(fields: &Object, name: &'static str) -> FieldResult<()> {
    match text(fields, name)? {
        Some(_) => Ok(()),
        None => Err(MediaError::Field { field: name }),
    }
}
