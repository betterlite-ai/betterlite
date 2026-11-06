use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::PayloadError;

pub(crate) const fn object<'json>(
    value: &'json Json,
    field: &'static str,
) -> Result<&'json IndexMap<String, Json>, PayloadError> {
    if let Json::Object(fields) = value {
        return Ok(fields);
    }
    Err(PayloadError::Field {
        field,
        expected: "a JSON object",
    })
}

pub(crate) fn optional_object<'json>(
    value: Option<&'json Json>,
    field: &'static str,
) -> Result<Option<&'json IndexMap<String, Json>>, PayloadError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(present) => object(present, field).map(Some),
    }
}

pub(crate) fn metadata_text<'json>(
    metadata: Option<&'json IndexMap<String, Json>>,
    key: &'static str,
) -> Result<Option<&'json str>, PayloadError> {
    let Some(value) = metadata.and_then(|fields| fields.get(key)) else {
        return Ok(None);
    };
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text)),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(PayloadError::MetadataText { key })
        }
    }
}
