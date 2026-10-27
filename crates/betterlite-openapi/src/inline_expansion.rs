use serde_json::{Map, Value};

use crate::schema_transforms::SchemaError;

const MAX_DEPTH: usize = 32;

pub fn inline_request_body(schema: Option<Value>, example: Value) -> Result<Value, SchemaError> {
    let media = serde_json::json!({"example": example});
    let Some(schema) = schema else {
        return Ok(serde_json::json!({"required": true, "content": {"application/json": media}}));
    };
    let definitions = schema
        .as_object()
        .and_then(|object| object.get("$defs"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let expanded = inline_value(schema, &definitions, 0)?;
    Ok(
        serde_json::json!({"required": true, "content": {"application/json": {"schema": expanded, "example": media["example"]}}}),
    )
}

fn inline_value(
    value: Value,
    definitions: &Map<String, Value>,
    depth: usize,
) -> Result<Value, SchemaError> {
    if depth >= MAX_DEPTH || !value.is_object() && !value.is_array() {
        return Ok(value);
    }
    if let Value::Array(values) = value {
        return values
            .into_iter()
            .map(|item| inline_value(item, definitions, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    let Value::Object(object) = value else {
        return Ok(value);
    };
    if let Some(reference) = object
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|value| value.strip_prefix("#/$defs/"))
    {
        let target = definitions
            .get(reference)
            .ok_or_else(|| SchemaError::MissingDefinition(reference.to_owned()))?;
        let target = inline_value(target.clone(), definitions, depth + 1)?;
        let siblings = object
            .into_iter()
            .filter(|(key, _)| key != "$ref")
            .map(|(key, value)| Ok((key, inline_value(value, definitions, depth + 1)?)))
            .collect::<Result<Map<_, _>, SchemaError>>()?;
        return Ok(match target {
            Value::Object(mut target) => {
                target.extend(siblings);
                Value::Object(target)
            }
            _ => Value::Object(siblings),
        });
    }
    if object.contains_key("propertyName") && object.contains_key("mapping") {
        let property_name = object.get("propertyName").cloned().unwrap_or(Value::Null);
        return Ok(serde_json::json!({"propertyName": property_name}));
    }
    object
        .into_iter()
        .filter(|(key, _)| key != "$defs")
        .map(|(key, value)| Ok((key, inline_value(value, definitions, depth + 1)?)))
        .collect::<Result<Map<_, _>, SchemaError>>()
        .map(Value::Object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_reference_and_removes_defs() {
        let body = inline_request_body(
            Some(serde_json::json!({"$defs":{"Message":{"type":"string"}},"properties":{"message":{"$ref":"#/$defs/Message"}}})),
            serde_json::json!({"message":"ok"}),
        ).expect("inline schema should be valid");
        assert_eq!(
            body["content"]["application/json"]["schema"]["properties"]["message"]["type"],
            "string"
        );
        assert!(
            body["content"]["application/json"]["schema"]
                .get("$defs")
                .is_none()
        );
    }

    #[test]
    fn missing_reference_is_reported() {
        let result = inline_request_body(
            Some(serde_json::json!({"$ref":"#/$defs/Missing"})),
            serde_json::json!({}),
        );
        assert!(matches!(result, Err(SchemaError::MissingDefinition(name)) if name == "Missing"));
    }
}
