use secrecy::SecretString;
use serde_json::Value;

use crate::chat::{ChatMessage, MessageContent};
use crate::nlp_cloud_chat::NlpCloudConfig;
use crate::nlp_cloud_chat::error::NlpCloudError;
use crate::nlp_cloud_chat::headers::validate_environment;
use crate::openai_base_config::{Headers, Object};

pub struct CompletionInput<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub api_base: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub caller_headers: &'request Headers,
    pub optional_params: &'request Object,
    pub config: &'request NlpCloudConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionRequest {
    pub url: String,
    pub headers: Headers,
    pub body: Object,
}

pub fn completion_request(input: &CompletionInput<'_>) -> Result<CompletionRequest, NlpCloudError> {
    let mut options = input.optional_params.clone();
    input.config.merge_defaults(&mut options);
    Ok(CompletionRequest {
        url: format!("{}{}/generation", input.api_base, input.model),
        headers: validate_environment(input.caller_headers, input.api_key),
        body: transform_request(input.messages, &options)?,
    })
}

pub fn transform_request(
    messages: &[ChatMessage],
    optional_params: &Object,
) -> Result<Object, NlpCloudError> {
    let mut body = Object::from_iter([("text".to_owned(), Value::String(message_text(messages)?))]);
    body.extend(optional_params.clone());
    Ok(body)
}

fn message_text(messages: &[ChatMessage]) -> Result<String, NlpCloudError> {
    let mut text = String::new();
    for (index, message) in messages.iter().enumerate() {
        if index > 0 {
            text.push(' ');
        }
        match &message.content {
            None => {}
            Some(MessageContent::Text(content)) => text.push_str(content),
            Some(MessageContent::Parts(_)) => {
                return Err(NlpCloudError {
                    status_code: 400,
                    message: format!("message {index} content is not a plain string"),
                    headers: None,
                });
            }
        }
    }
    Ok(text)
}
