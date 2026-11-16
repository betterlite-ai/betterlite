use crate::api::image_contracts::records::ImageData;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Number, de::Deserializer as JsonDeserializer, error::Category};
use std::str::FromStr as _;
use thiserror::Error;
type Shape<T> = Result<T, ReplyDecodeError>;
type Fields = IndexMap<String, Json>;
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
    usage: Option<Usage>,
    model: Option<Json>,
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
pub fn decode_openrouter_image_reply(bytes: &[u8]) -> Shape<OpenRouterImageReply> {
    // the source parses the whole body before it reads a field, so a syntax error beats a shape error
    let body: Json =
        serde_json::from_slice(bytes).map_err(|source| ReplyDecodeError::Json { source })?;
    let root = check_reply_shape(&body)?;
    let reply: Reply = serde_path_to_error::deserialize(&mut JsonDeserializer::from_slice(bytes))
        .map_err(|error| match error.inner().classify() {
        Category::Data => ReplyDecodeError::Shape {
            message: error.to_string(),
        },
        Category::Io | Category::Syntax | Category::Eof => ReplyDecodeError::Json {
            source: error.into_inner(),
        },
    })?;
    Ok(OpenRouterImageReply {
        images: images(root)?,
        usage: usage_record(root, reply.usage)?,
        model: reply
            .model
            .or_else(|| root.contains_key("model").then_some(Json::Null)),
    })
}
// a serde derive also reads a list as a struct, where the source's .get() raises, so objects are checked first
fn check_reply_shape(body: &Json) -> Shape<&Fields> {
    let root = object(body, "")?;
    if let Some(model) = root.get("model")
        && !matches!(model, Json::Null | Json::Text(_))
    {
        return Err(invalid_type("model", model, "a string"));
    }
    if let Some(usage) = root.get("usage") {
        check_usage(usage, "usage")?;
    }
    Ok(root)
}
fn images(root: &Fields) -> Shape<Vec<ImageData>> {
    let mut images = Vec::new();
    let Some(choices) = root.get("choices") else {
        return Ok(images);
    };
    for (choice_index, choice) in list(choices, "choices")?.iter().enumerate() {
        let choice_path = format!("choices[{choice_index}]");
        let Some(message) = object(choice, &choice_path)?.get("message") else {
            continue;
        };
        let message_path = format!("{choice_path}.message");
        let Some(entries) = object(message, &message_path)?.get("images") else {
            continue;
        };
        let images_path = format!("{message_path}.images");
        for (entry_index, entry) in list(entries, &images_path)?.iter().enumerate() {
            let entry_path = format!("{images_path}[{entry_index}]");
            let Some(url) = object(entry, &entry_path)?.get("image_url") else {
                continue;
            };
            let url_path = format!("{entry_path}.image_url");
            match object(url, &url_path)?.get("url") {
                Some(Json::Text(link)) if !link.is_empty() => images.push(image_data(link)),
                Some(Json::Text(_) | Json::Null) | None => {}
                Some(found) => {
                    return Err(invalid_type(&format!("{url_path}.url"), found, "a string"));
                }
            }
        }
    }
    Ok(images)
}
fn check_usage(value: &Json, path: &str) -> Shape<()> {
    let Some(usage) = optional_object(value, path)? else {
        return Ok(());
    };
    for name in ["completion_tokens_details", "prompt_tokens_details"] {
        if let Some(details) = usage.get(name) {
            optional_object(details, &format!("{path}.{name}"))?;
        }
    }
    Ok(())
}
fn list<'reply>(value: &'reply Json, path: &str) -> Shape<&'reply [Json]> {
    let Json::List(items) = value else {
        return Err(invalid_type(path, value, "a list"));
    };
    Ok(items)
}
fn object<'reply>(value: &'reply Json, path: &str) -> Shape<&'reply Fields> {
    let Json::Object(fields) = value else {
        return Err(invalid_type(path, value, "a JSON object"));
    };
    Ok(fields)
}
fn optional_object<'reply>(value: &'reply Json, path: &str) -> Shape<Option<&'reply Fields>> {
    match value {
        Json::Null => Ok(None),
        Json::Object(fields) => Ok(Some(fields)),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(invalid_type(path, value, "an object"))
        }
    }
}
fn invalid_type(path: &str, value: &Json, expected: &str) -> ReplyDecodeError {
    let found = match value {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::Text(_) => "a string",
        Json::List(_) => "a list",
        Json::Object(_) => "an object",
    };
    let message = if path.is_empty() {
        format!("invalid type: {found}, expected {expected}")
    } else {
        format!("{path}: invalid type: {found}, expected {expected}")
    };
    ReplyDecodeError::Shape { message }
}
fn image_data(url: &str) -> ImageData {
    url.strip_prefix("data:").map_or_else(
        || ImageData {
            b64_json: None,
            url: Some(url.to_owned()),
            revised_prompt: None,
        },
        |data| ImageData {
            b64_json: data.split_once(',').map(|(_, encoded)| encoded.to_owned()),
            url: None,
            revised_prompt: None,
        },
    )
}
// an absent usage and an empty one are both no usage, as the source's `if usage` reads them
fn usage_record(root: &Fields, parsed: Option<Usage>) -> Shape<Option<OpenRouterImageUsage>> {
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
        output_image_tokens: image_tokens(counts.completion_tokens_details),
        input_image_tokens: image_tokens(counts.prompt_tokens_details),
        cost: counts.cost.as_ref().map(decimal_cost).transpose()?,
        cost_details: counts.cost_details.unwrap_or_default(),
    }))
}
fn image_tokens(details: Option<TokenDetails>) -> i64 {
    details
        .and_then(|counted| counted.image_tokens)
        .unwrap_or_default()
}
fn decimal_cost(number: &Number) -> Shape<Decimal> {
    let text = number.to_string();
    let Ok(cost) = Decimal::from_str(&text) else {
        return Err(ReplyDecodeError::Shape {
            message: format!("usage.cost: {text} does not fit a decimal cost"),
        });
    };
    Ok(cost)
}
