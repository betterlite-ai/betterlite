use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::{ChatContent, Mode, UrlDownloader};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;

use super::env::{PROVIDER, STORED_MODEL_FIELD};
use super::error::RagflowError;
use super::parse::parse_model;

pub struct RequestInput<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request Object,
    pub litellm_params: &'request Object,
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}

mod tests;

// LiteLLM's transform_request: the stored model wins, then the parsed one, then the whole model string
pub fn transform_request(input: &RequestInput<'_>) -> Result<Object, RagflowError> {
    let actual_model = actual_model(input.model, input.litellm_params)?;
    let messages = chat_messages(input.messages)?;
    let transformed = ChatContent {
        model: &actual_model,
        messages: &messages,
        optional_params: input.optional_params,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: input.environment,
        downloads: input.downloads,
    }
    .transform(Mode::Sync)?;
    Ok(transformed.to_body()?)
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

fn chat_messages(messages: &[Message]) -> Result<Vec<ChatMessage>, RagflowError> {
    let value =
        serde_json::to_value(messages).map_err(|source| RagflowError::MessageShape { source })?;
    serde_json::from_value(value).map_err(|source| RagflowError::MessageShape { source })
}
