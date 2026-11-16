use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role};
use serde::Serialize;

use crate::api::fal::vision_chat::error::FalAIError;
use crate::api::fal::vision_chat::optional_params::Params;

const PASSTHROUGH_PARAMS: [&str; 3] = ["reasoning", "temperature", "top_p"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FalChatBody {
    pub prompt: String,
    pub image_url: String,
    #[serde(flatten)]
    pub options: Params,
}

// the stream check runs before the messages are read, and only the three passthrough options survive
pub fn transform_request(
    messages: &[ChatMessage],
    optional_params: &Params,
) -> Result<FalChatBody, FalAIError> {
    match optional_params.get("stream") {
        Some(Json::Bool(true)) => return Err(FalAIError::StreamingUnsupported),
        Some(Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(FalAIError::StreamNotBoolean);
        }
        Some(Json::Bool(false) | Json::Null) | None => {}
    }
    let (prompt, image_url) = prompt_and_image(messages)?;
    let options = optional_params
        .iter()
        .filter(|(name, value)| {
            PASSTHROUGH_PARAMS.contains(&name.as_str()) && !matches!(value, Json::Null)
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Ok(FalChatBody {
        prompt,
        image_url,
        options,
    })
}

fn prompt_and_image(messages: &[ChatMessage]) -> Result<(String, String), FalAIError> {
    let [message] = messages else {
        return Err(FalAIError::OneUserMessage);
    };
    if message.role != Role::User {
        return Err(FalAIError::OneUserMessage);
    }
    let parts = match &message.content {
        Some(MessageContent::Text(text)) if text.is_empty() => {
            return Err(FalAIError::TextRequired);
        }
        Some(MessageContent::Text(_)) => return Err(FalAIError::OneImageRequired),
        Some(MessageContent::Parts(parts)) => parts.as_slice(),
        None => &[],
    };
    let (texts, image_urls) = split_parts(parts);
    if texts.is_empty() {
        return Err(FalAIError::TextRequired);
    }
    match image_urls.as_slice() {
        [url] => Ok((texts.join("\n"), (*url).to_owned())),
        _ => Err(FalAIError::OneImageRequired),
    }
}

fn split_parts(parts: &[ContentPart]) -> (Vec<&str>, Vec<&str>) {
    let mut texts = Vec::new();
    let mut image_urls = Vec::new();
    for part in parts {
        match part.kind.as_str() {
            "text" => {
                if let Some(value) = part.fields.get("text")
                    && let Some(text) = value.as_str()
                    && !text.is_empty()
                {
                    texts.push(text);
                }
            }
            "image_url" => image_urls.extend(image_part_url(part)),
            _ => {}
        }
    }
    (texts, image_urls)
}

// an image_url that is neither a string nor an object with a string url is not an image part
fn image_part_url(part: &ContentPart) -> Option<&str> {
    let image_url = part.fields.get("image_url")?;
    if let Some(url) = image_url.as_str() {
        return Some(url);
    }
    image_url.get("url")?.as_str()
}
