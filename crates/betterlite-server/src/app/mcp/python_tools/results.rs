use betterlite_mcp::openapi_tools::output::OpenApiToolOutput;
use betterlite_prompts::arguments::json::Json;
use betterlite_pyhost::json_values::JsonValue;

// LiteLLM's handler_outcome wraps every return in str(); betterlite keeps the string and the value's structure
pub(super) fn tool_output(value: JsonValue) -> OpenApiToolOutput {
    match value {
        JsonValue::String(text) => OpenApiToolOutput::Text(text),
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)) => {
            let original_text = other.to_string();
            OpenApiToolOutput::Json {
                value: prompt_value(other),
                original_text,
            }
        }
    }
}

fn prompt_value(value: JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(flag),
        JsonValue::Number(number) => Json::Number(number),
        JsonValue::String(text) => Json::Text(text),
        JsonValue::Array(items) => Json::List(items.into_iter().map(prompt_value).collect()),
        JsonValue::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(name, item)| (name, prompt_value(item)))
                .collect(),
        ),
    }
}
