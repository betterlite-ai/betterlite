use betterlite_openai::openai_base_config::{
    ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use betterlite_prompts::message_content::Message;
use thiserror::Error;

use super::messages::{self, MessagesError};

#[derive(Debug, Error)]
pub enum RequestError {
    #[error(transparent)]
    Messages(#[from] MessagesError),
    #[error("the transformed messages could not be encoded")]
    Json(#[from] serde_json::Error),
}

pub fn transform_request(
    model: &str,
    messages: &[Message],
    optional_params: &Object,
) -> Result<Object, RequestError> {
    let mut params = optional_params.clone();
    // LiteLLM pops extra_body, so a truthy non-object value is dropped rather than sent
    if let Some(extra) = params.remove("extra_body")
        && let Ok(merged) = serde_json::from_value::<Object>(extra)
        && !merged.is_empty()
    {
        params.extend(merged);
    }
    params.remove("max_retries");
    let transformed = messages::transform_messages(messages)?;
    let encoded = serde_json::to_value(transformed)?;
    let config = OpenAiConfig {
        family: ConfigFamily::AzureAiStudio,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    };
    Ok(config.request_body(model, encoded, &params))
}
