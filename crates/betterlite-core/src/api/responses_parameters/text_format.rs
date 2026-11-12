use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

use super::error::ResponsesParametersError;

pub fn text_format_to_text(
    text_format: Option<&JsonValue>,
    text: Option<&JsonValue>,
) -> Result<Option<JsonValue>, ResponsesParametersError> {
    let Some(given_format) = text_format else {
        return Ok(text.cloned());
    };
    if text.is_some() {
        return Ok(text.cloned());
    }
    let JsonValue::Object(format) = given_format else {
        return Err(ResponsesParametersError::TextFormatNotObject);
    };
    let Some(format_type) = format.get("type") else {
        return Err(ResponsesParametersError::MissingTextFormatField { field: "type" });
    };
    let Some(JsonValue::Object(envelope)) = format.get("json_schema") else {
        return Err(ResponsesParametersError::MissingTextFormatField {
            field: "json_schema",
        });
    };
    let Some(name) = envelope.get("name") else {
        return Err(ResponsesParametersError::MissingTextFormatField {
            field: "json_schema.name",
        });
    };
    let Some(schema) = envelope.get("schema") else {
        return Err(ResponsesParametersError::MissingTextFormatField {
            field: "json_schema.schema",
        });
    };
    let Some(strict) = envelope.get("strict") else {
        return Err(ResponsesParametersError::MissingTextFormatField {
            field: "json_schema.strict",
        });
    };
    let mut shape = IndexMap::with_capacity(4);
    shape.insert("type".to_owned(), format_type.clone());
    shape.insert("name".to_owned(), name.clone());
    shape.insert("schema".to_owned(), schema.clone());
    shape.insert("strict".to_owned(), strict.clone());
    let mut converted = IndexMap::with_capacity(1);
    converted.insert("format".to_owned(), JsonValue::Object(shape));
    Ok(Some(JsonValue::Object(converted)))
}
