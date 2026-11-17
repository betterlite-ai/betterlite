use serde_json::{Map, Value};

use super::error::RequestError;

pub fn responses_text_format(response_format: &Value) -> Result<Option<Value>, RequestError> {
    let Value::Object(fields) = response_format else {
        return Ok(None);
    };
    let format = match fields.get("type").and_then(Value::as_str) {
        Some("json_schema") => json_schema_format(fields)?,
        Some("json_object") => typed_format("json_object"),
        Some("text") => typed_format("text"),
        _ => return Ok(None),
    };
    Ok(Some(format))
}

fn typed_format(kind: &str) -> Value {
    let mut format = Map::new();
    format.insert("type".to_owned(), Value::String(kind.to_owned()));
    Value::Object(format)
}

fn json_schema_format(fields: &Map<String, Value>) -> Result<Value, RequestError> {
    let absent = Map::new();
    let schema = match fields.get("json_schema") {
        None => &absent,
        Some(Value::Object(schema)) => schema,
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_),
        ) => {
            return Err(RequestError::FieldType {
                field: "response_format",
                expected: "a json_schema object",
            });
        }
    };
    let mut format = Map::new();
    format.insert("type".to_owned(), Value::String("json_schema".to_owned()));
    format.insert(
        "name".to_owned(),
        schema
            .get("name")
            .cloned()
            .unwrap_or_else(|| Value::String("response_schema".to_owned())),
    );
    format.insert(
        "schema".to_owned(),
        schema
            .get("schema")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new())),
    );
    format.insert(
        "strict".to_owned(),
        schema.get("strict").cloned().unwrap_or(Value::Bool(false)),
    );
    Ok(Value::Object(format))
}

pub fn merge_text(
    params: &mut Map<String, Value>,
    key: &str,
    value: Value,
) -> Result<(), RequestError> {
    let mut text = match params.get("text") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(text)) => text.clone(),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
            return Err(RequestError::FieldType {
                field: "text",
                expected: "an object",
            });
        }
    };
    text.insert(key.to_owned(), value);
    params.insert("text".to_owned(), Value::Object(text));
    Ok(())
}
