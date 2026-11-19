use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;

use super::env::STORED_MODEL_FIELD;
use super::error::RagflowError;
use super::parse::parse_model;

pub struct RequestInput<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request Object,
    pub litellm_params: &'request Object,
}

mod tests;

// LiteLLM's RAGFlowConfig.transform_request calls OpenAIConfig.transform_request, which merges {model, messages, **optional_params} untouched.
pub fn transform_request(input: &RequestInput<'_>) -> Result<Object, RagflowError> {
    let actual_model = actual_model(input.model, input.litellm_params)?;
    let messages: Json = serde_json::from_value(
        serde_json::to_value(input.messages)
            .map_err(|source| RagflowError::MessageShape { source })?,
    )
    .map_err(|source| RagflowError::MessageShape { source })?;
    let mut body = input.optional_params.clone();
    body.insert(String::from("model"), Json::Text(actual_model));
    body.insert(String::from("messages"), messages);
    Ok(body)
}

fn actual_model(model: &str, litellm_params: &Object) -> Result<String, RagflowError> {
    match litellm_params.get(STORED_MODEL_FIELD) {
        None | Some(Json::Null) => Ok(match parse_model(model) {
            Ok(parsed) => parsed.model,
            Err(_) => model.to_owned(),
        }),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(RagflowError::FieldType {
                field: STORED_MODEL_FIELD,
            })
        }
    }
}
