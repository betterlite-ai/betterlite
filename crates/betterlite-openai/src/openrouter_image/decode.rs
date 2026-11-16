use crate::api::image_contracts::records::ImageData;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum ReplyDecodeError {
    #[error("{source}")]
    Json { source: serde_json::Error },
    #[error("{0}")]
    Shape(&'static str),
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenRouterImageUsage {
    pub prompt_tokens: Option<Json>,
    pub total_tokens: Option<Json>,
    pub completion_tokens_details: Option<Json>,
    pub prompt_tokens_details: Option<Json>,
    pub cost: Option<Json>,
    pub cost_details: Option<Json>,
}
impl OpenRouterImageUsage {
    #[must_use]
    pub fn from_object(fields: &IndexMap<String, Json>) -> Self {
        Self {
            prompt_tokens: fields.get("prompt_tokens").cloned(),
            total_tokens: fields.get("total_tokens").cloned(),
            completion_tokens_details: fields.get("completion_tokens_details").cloned(),
            prompt_tokens_details: fields.get("prompt_tokens_details").cloned(),
            cost: fields.get("cost").cloned(),
            cost_details: fields.get("cost_details").cloned(),
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenRouterImageReply {
    pub images: Vec<ImageData>,
    pub usage: Option<OpenRouterImageUsage>,
    pub model: Option<Json>,
}
pub fn decode_openrouter_image_reply(
    bytes: &[u8],
) -> Result<OpenRouterImageReply, ReplyDecodeError> {
    let value: Json =
        serde_json::from_slice(bytes).map_err(|source| ReplyDecodeError::Json { source })?;
    let Json::Object(fields) = value else {
        return Err(ReplyDecodeError::Shape(
            "the response body must be an object",
        ));
    };
    Ok(OpenRouterImageReply {
        images: reply_images(&fields)?,
        usage: reply_usage(&fields)?,
        model: fields.get("model").cloned(),
    })
}
fn reply_images(fields: &IndexMap<String, Json>) -> Result<Vec<ImageData>, ReplyDecodeError> {
    let Some(choices) = fields.get("choices") else {
        return Ok(Vec::new());
    };
    let Json::List(choices) = choices else {
        return Err(ReplyDecodeError::Shape("choices must be a list"));
    };
    let mut images = Vec::new();
    for choice in choices {
        images.extend(choice_images(choice)?);
    }
    Ok(images)
}
fn choice_images(choice: &Json) -> Result<Vec<ImageData>, ReplyDecodeError> {
    let Json::Object(choice) = choice else {
        return Err(ReplyDecodeError::Shape("each choice must be an object"));
    };
    let Some(message) = choice.get("message") else {
        return Ok(Vec::new());
    };
    let Json::Object(message) = message else {
        return Err(ReplyDecodeError::Shape("message must be an object"));
    };
    let Some(entries) = message.get("images") else {
        return Ok(Vec::new());
    };
    let Json::List(entries) = entries else {
        return Err(ReplyDecodeError::Shape("images must be a list"));
    };
    let mut images = Vec::new();
    for entry in entries {
        if let Some(image) = image_from_entry(entry)? {
            images.push(image);
        }
    }
    Ok(images)
}
fn image_from_entry(entry: &Json) -> Result<Option<ImageData>, ReplyDecodeError> {
    let Json::Object(entry) = entry else {
        return Err(ReplyDecodeError::Shape("each image must be an object"));
    };
    let Some(image_url) = entry.get("image_url") else {
        return Ok(None);
    };
    let Json::Object(image_url) = image_url else {
        return Err(ReplyDecodeError::Shape("image_url must be an object"));
    };
    match image_url.get("url") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(url)) if url.is_empty() => Ok(None),
        Some(Json::Text(url)) => Ok(Some(image_data(url))),
        Some(_) => Err(ReplyDecodeError::Shape("image_url.url must be a string")),
    }
}
fn image_data(url: &str) -> ImageData {
    match url.strip_prefix("data:") {
        Some(data) => ImageData {
            b64_json: data.split_once(',').map(|(_, encoded)| encoded.to_owned()),
            url: None,
            revised_prompt: None,
        },
        None => ImageData {
            b64_json: None,
            url: Some(url.to_owned()),
            revised_prompt: None,
        },
    }
}
fn reply_usage(
    fields: &IndexMap<String, Json>,
) -> Result<Option<OpenRouterImageUsage>, ReplyDecodeError> {
    match fields.get("usage") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(usage)) if usage.is_empty() => Ok(None),
        Some(Json::Object(usage)) => Ok(Some(OpenRouterImageUsage::from_object(usage))),
        Some(_) => Err(ReplyDecodeError::Shape("usage must be an object")),
    }
}
