use betterlite_openai::chat::response::ChatResponse;
use betterlite_prompts::arguments::json::Json;
use thiserror::Error;

use super::AZURE_AI_PREFIX;

#[derive(Debug, Error)]
pub enum ResponseError {
    #[error("the response body is not a JSON object")]
    NotAnObject,
    #[error("the response model must be a string")]
    ModelNotString,
    #[error("the response body does not match the chat response shape")]
    Json(#[from] serde_json::Error),
}

// LiteLLM assigns azure_ai/<requested> before the shared conversion, which then prefixes the body's model
pub fn transform_response(model: &str, body: Json) -> Result<ChatResponse, ResponseError> {
    let Json::Object(mut fields) = body else {
        return Err(ResponseError::NotAnObject);
    };
    let assigned = format!("{AZURE_AI_PREFIX}{model}");
    let resolved = match fields.get("model") {
        None | Some(Json::Null) => assigned,
        Some(Json::Text(provider_model)) => format!("{AZURE_AI_PREFIX}{provider_model}"),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(ResponseError::ModelNotString);
        }
    };
    fields.insert(String::from("model"), Json::Text(resolved));
    let encoded = serde_json::to_value(Json::Object(fields))?;
    Ok(serde_json::from_value(encoded)?)
}
