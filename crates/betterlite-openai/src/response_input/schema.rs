use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolSchemaError {
    #[error("the tool schema is not valid JSON: {source}")]
    Json {
        #[source]
        source: PrivateJsonError,
    },
    #[error("the tool schema decodes to a value that is not an object")]
    NotObject,
}

pub fn decode_response_tool_schema(bytes: &[u8]) -> Result<Object, ToolSchemaError> {
    let value = match safe_json_loads(bytes) {
        Ok(value) => value,
        Err(source) => return Err(ToolSchemaError::Json { source }),
    };
    let Ok(Json::Object(object)) = serde_json::from_value::<Json>(value) else {
        return Err(ToolSchemaError::NotObject);
    };
    Ok(object)
}
