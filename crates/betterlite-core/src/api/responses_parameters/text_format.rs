use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

use super::error::ResponsesParametersError;

pub fn text_format_to_text(
    text_format: Option<&JsonValue>,
    text: Option<&JsonValue>,
) -> Result<Option<JsonValue>, ResponsesParametersError> {
    let Some(given_format) = text_format.filter(|_format| text.is_none()) else {
        return Ok(text.cloned());
    };
    let JsonValue::Object(format) = given_format else {
        return Err(ResponsesParametersError::TextFormatNotObject);
    };
    let JsonValue::Object(envelope) = required(format, "json_schema")? else {
        return Err(ResponsesParametersError::TextFormatNotObject);
    };
    let shape = IndexMap::from([
        ("type".to_owned(), required(format, "type")?.clone()),
        (
            "name".to_owned(),
            required(envelope, "json_schema.name")?.clone(),
        ),
        (
            "schema".to_owned(),
            required(envelope, "json_schema.schema")?.clone(),
        ),
        (
            "strict".to_owned(),
            required(envelope, "json_schema.strict")?.clone(),
        ),
    ]);
    let converted = IndexMap::from([("format".to_owned(), JsonValue::Object(shape))]);
    Ok(Some(JsonValue::Object(converted)))
}

fn required<'value>(
    fields: &'value IndexMap<String, JsonValue>,
    field: &'static str,
) -> Result<&'value JsonValue, ResponsesParametersError> {
    fields
        .get(field.rsplit_once('.').map_or(field, |(_, key)| key))
        .ok_or(ResponsesParametersError::MissingTextFormatField { field })
}
