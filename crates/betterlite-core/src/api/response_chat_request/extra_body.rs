use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::declared::declared_key;
use super::error::RequestError;

pub fn extract_extra_body_params(
    optional_params: &mut IndexMap<String, Json>,
) -> Result<(), RequestError> {
    // LiteLLM always pops extra_body, so a falsy one leaves the options without it
    let Some(extra_body) = optional_params.shift_remove("extra_body") else {
        return Ok(());
    };
    let fields = match extra_body {
        Json::Null => return Ok(()),
        Json::Object(fields) => fields,
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(RequestError::FieldType {
                field: "extra_body",
                expected: "an object",
            });
        }
    };
    // Json keeps extra_body's own order, and LiteLLM appends these in the order the caller wrote them
    for (key, value) in fields {
        // previous_response_id is a declared key already, and reasoning_effort is the other one LiteLLM maps
        if declared_key(&key) || key == "reasoning_effort" {
            optional_params.insert(key, value);
        }
    }
    Ok(())
}
