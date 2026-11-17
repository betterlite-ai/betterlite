use indexmap::IndexMap;
use serde_json::Value;

use super::declared::declared_key;
use super::error::RequestError;

pub fn extract_extra_body_params(
    optional_params: &mut IndexMap<String, Value>,
) -> Result<(), RequestError> {
    // LiteLLM always pops extra_body, so a falsy one leaves the options without it
    let Some(extra_body) = optional_params.shift_remove("extra_body") else {
        return Ok(());
    };
    let fields = match extra_body {
        Value::Null => return Ok(()),
        Value::Object(fields) => fields,
        Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            return Err(RequestError::FieldType {
                field: "extra_body",
                expected: "an object",
            });
        }
    };
    for (key, value) in fields {
        // previous_response_id is a declared key already, and reasoning_effort is the other one LiteLLM maps
        if declared_key(&key) || key == "reasoning_effort" {
            optional_params.insert(key, value);
        }
    }
    Ok(())
}
