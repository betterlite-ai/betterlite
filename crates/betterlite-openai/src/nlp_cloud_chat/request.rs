use serde_json::Value;

use crate::chat::{ChatMessage, MessageContent};
use crate::nlp_cloud_chat::error::NlpCloudError;
use crate::openai_base_config::Object;

#[must_use]
pub fn complete_url(api_base: &str, model: &str) -> String {
    let capacity = api_base
        .len()
        .saturating_add(model.len())
        .saturating_add("/generation".len());
    let mut url = String::with_capacity(capacity);
    url.push_str(api_base);
    url.push_str(model);
    url.push_str("/generation");
    url
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
