use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Number;
use serde_json::de::Deserializer as JsonDeserializer;
use serde_json::error::Category;
use serde_with::rust::double_option;
use thiserror::Error;

use crate::api::image_contracts::records::ImageData;

#[derive(Debug, Error)]
pub enum ReplyDecodeError {
    #[error("{source}")]
    Json { source: serde_json::Error },
    #[error("{message}")]
    Shape { message: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenRouterImageUsage {
    pub prompt_tokens: i64,
    pub total_tokens: i64,
    pub output_image_tokens: i64,
    pub input_image_tokens: i64,
    pub cost: Option<Decimal>,
    pub cost_details: IndexMap<String, Json>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenRouterImageReply {
    pub images: Vec<ImageData>,
    pub usage: Option<OpenRouterImageUsage>,
    pub model: Option<Json>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Reply {
    choices: Vec<Choice>,
    usage: Option<Usage>,
    #[serde(with = "double_option")]
    model: Option<Option<String>>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Choice {
    message: Message,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Message {
    images: Vec<Entry>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Entry {
    image_url: ImageUrl,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct ImageUrl {
    url: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Usage {
    prompt_tokens: Option<i64>,
    total_tokens: Option<i64>,
    completion_tokens_details: Option<TokenDetails>,
    prompt_tokens_details: Option<TokenDetails>,
    cost: Option<Number>,
    cost_details: Option<IndexMap<String, Json>>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TokenDetails {
    image_tokens: Option<i64>,
}

pub fn decode_openrouter_image_reply(
    bytes: &[u8],
) -> Result<OpenRouterImageReply, ReplyDecodeError> {
    // the source parses the whole body before it reads a field, so a syntax error beats a shape error
    let body: Json =
        serde_json::from_slice(bytes).map_err(|source| ReplyDecodeError::Json { source })?;
    let root = check_reply_shape(&body)?;
    let reply: Reply = serde_path_to_error::deserialize(&mut JsonDeserializer::from_slice(bytes))
        .map_err(|error| match error.inner().classify() {
        Category::Data => ReplyDecodeError::Shape {
            message: error.to_string(),
        },
        _ => ReplyDecodeError::Json {
            source: error.into_inner(),
        },
    })?;
    Ok(OpenRouterImageReply {
        images: image_links(reply.choices),
        usage: usage_record(root, reply.usage)?,
        model: reply
            .model
            .map(|model| model.map_or(Json::Null, Json::Text)),
    })
}

// a serde derive also reads a list as a struct, where the source's .get() raises, so objects are checked first
fn check_reply_shape(body: &Json) -> Result<&IndexMap<String, Json>, ReplyDecodeError> {
    let root = object(body, "")?;
    if let Some(choices) = root.get("choices") {
        check_choices(choices, "choices")?;
    }
    if let Some(usage) = root.get("usage") {
        check_usage(usage, "usage")?;
    }
    Ok(root)
}

fn check_list<'reply>(value: &'reply Json, path: &str) -> Result<&'reply [Json], ReplyDecodeError> {
    let Json::List(items) = value else {
        return Err(invalid_type(path, value, "a list"));
    };
    Ok(items)
}

fn check_choices(value: &Json, path: &str) -> Result<(), ReplyDecodeError> {
    for (index, choice) in check_list(value, path)?.iter().enumerate() {
        check_choice(choice, &format!("{path}[{index}]"))?;
    }
    Ok(())
}

fn check_choice(value: &Json, path: &str) -> Result<(), ReplyDecodeError> {
    let choice = object(value, path)?;
    if let Some(message) = choice.get("message") {
        check_message(message, &format!("{path}.message"))?;
    }
    Ok(())
}

fn check_message(value: &Json, path: &str) -> Result<(), ReplyDecodeError> {
    let message = object(value, path)?;
    if let Some(images) = message.get("images") {
        check_images(images, &format!("{path}.images"))?;
    }
    Ok(())
}

fn check_images(value: &Json, path: &str) -> Result<(), ReplyDecodeError> {
    for (index, entry) in check_list(value, path)?.iter().enumerate() {
        check_entry(entry, &format!("{path}[{index}]"))?;
    }
    Ok(())
}

fn check_entry(value: &Json, path: &str) -> Result<(), ReplyDecodeError> {
    let entry = object(value, path)?;
    if let Some(image_url) = entry.get("image_url") {
        object(image_url, &format!("{path}.image_url"))?;
    }
    Ok(())
}

fn check_usage(value: &Json, path: &str) -> Result<(), ReplyDecodeError> {
    match value {
        Json::Null => Ok(()),
        Json::Object(usage) => check_usage_details(usage, path),
        _ => Err(invalid_type(path, value, "an object")),
    }
}

fn check_usage_details(usage: &IndexMap<String, Json>, path: &str) -> Result<(), ReplyDecodeError> {
    for name in ["completion_tokens_details", "prompt_tokens_details"] {
        if let Some(details) = usage.get(name) {
            check_optional_object(details, &format!("{path}.{name}"))?;
        }
    }
    Ok(())
}

fn check_optional_object(value: &Json, path: &str) -> Result<(), ReplyDecodeError> {
    if let Json::Null = value {
        return Ok(());
    }
    object(value, path)?;
    Ok(())
}

fn object<'reply>(
    value: &'reply Json,
    path: &str,
) -> Result<&'reply IndexMap<String, Json>, ReplyDecodeError> {
    let Json::Object(fields) = value else {
        return Err(invalid_type(path, value, "a JSON object"));
    };
    Ok(fields)
}

fn invalid_type(path: &str, value: &Json, expected: &str) -> ReplyDecodeError {
    let found = found_type(value);
    let message = if path.is_empty() {
        format!("invalid type: {found}, expected {expected}")
    } else {
        format!("{path}: invalid type: {found}, expected {expected}")
    };
    ReplyDecodeError::Shape { message }
}

const fn found_type(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::Text(_) => "a string",
        Json::List(_) => "a list",
        Json::Object(_) => "an object",
    }
}

fn image_links(choices: Vec<Choice>) -> Vec<ImageData> {
    let mut images = Vec::new();
    for choice in choices {
        for entry in choice.message.images {
            if let Some(link) = entry.image_url.url.filter(|link| !link.is_empty()) {
                images.push(image_data(&link));
            }
        }
    }
    images
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

// an absent usage and an empty one are both no usage, as the source's `if usage` reads them
fn usage_record(
    root: &IndexMap<String, Json>,
    parsed: Option<Usage>,
) -> Result<Option<OpenRouterImageUsage>, ReplyDecodeError> {
    let Some(Json::Object(usage)) = root.get("usage") else {
        return Ok(None);
    };
    if usage.is_empty() {
        return Ok(None);
    }
    let counts = parsed.unwrap_or_default();
    Ok(Some(OpenRouterImageUsage {
        prompt_tokens: counts.prompt_tokens.unwrap_or_default(),
        total_tokens: counts.total_tokens.unwrap_or_default(),
        output_image_tokens: details_image_tokens(counts.completion_tokens_details),
        input_image_tokens: details_image_tokens(counts.prompt_tokens_details),
        cost: counts.cost.map(decimal_cost).transpose()?,
        cost_details: counts.cost_details.unwrap_or_default(),
    }))
}

fn details_image_tokens(details: Option<TokenDetails>) -> i64 {
    details
        .and_then(|counted| counted.image_tokens)
        .unwrap_or_default()
}

fn decimal_cost(number: Number) -> Result<Decimal, ReplyDecodeError> {
    let text = number.to_string();
    let Ok(cost) = Decimal::from_str(&text) else {
        return Err(ReplyDecodeError::Shape {
            message: format!("usage.cost: {text} does not fit a decimal cost"),
        });
    };
    Ok(cost)
}
