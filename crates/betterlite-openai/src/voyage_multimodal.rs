use serde_json::{Map, Value};
use thiserror::Error;

use crate::voyage_embedding::voyage_api_key;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["dimensions"];

#[derive(Debug, Error)]
pub enum VoyageMultimodalError {
    #[error(
        "Voyage multimodal embeddings require a non-empty `image_url`. Got an image content block without a `url`."
    )]
    MissingImageUrl,
    #[error("the Voyage multimodal content must be a list of content blocks")]
    ContentNotList,
    #[error("a Voyage multimodal content item must be an object")]
    ContentItemNotObject,
    #[error(
        "Voyage API key is required for multimodal embeddings. Set VOYAGE_API_KEY / VOYAGE_AI_API_KEY / VOYAGE_AI_TOKEN or pass `api_key` explicitly."
    )]
    MissingApiKey,
}

#[must_use]
pub fn is_multimodal_embeddings(model: &str) -> bool {
    model.to_lowercase().contains("multimodal")
}

// LiteLLM rejects a falsey resolved key, unlike the text embedding leaf which sends "Bearer None"
pub fn credentials<F>(explicit: Option<&str>, lookup: F) -> Result<String, VoyageMultimodalError>
where
    F: Fn(&str) -> Option<String>,
{
    voyage_api_key(explicit, lookup)
        .filter(|key| !key.is_empty())
        .ok_or(VoyageMultimodalError::MissingApiKey)
}

pub fn request(
    model: &str,
    input: Value,
    options: &Map<String, Value>,
) -> Result<Value, VoyageMultimodalError> {
    let items = match input {
        Value::Array(items) => items,
        scalar => vec![scalar],
    };
    let normalized = items
        .into_iter()
        .map(normalize_input_item)
        .collect::<Result<Vec<_>, _>>()?;
    let mut result = Map::new();
    result.insert("inputs".to_owned(), Value::Array(normalized));
    result.insert("model".to_owned(), Value::String(model.to_owned()));
    let mut mapped = options.clone();
    if let Some(dimensions) = mapped.remove("dimensions") {
        mapped.insert("output_dimension".to_owned(), dimensions);
    }
    result.extend(mapped);
    Ok(Value::Object(result))
}

fn normalize_input_item(item: Value) -> Result<Value, VoyageMultimodalError> {
    match item {
        Value::String(text) => Ok(Value::Object(Map::from_iter([(
            "content".to_owned(),
            Value::Array(vec![Value::Object(Map::from_iter([
                ("type".to_owned(), Value::String("text".to_owned())),
                ("text".to_owned(), Value::String(text)),
            ]))]),
        )]))),
        Value::Object(mut fields) => {
            let Some(content) = fields.get("content") else {
                return Ok(Value::Object(fields));
            };
            let normalized = normalize_content(content)?;
            fields.insert("content".to_owned(), normalized);
            Ok(Value::Object(fields))
        }
        other => Ok(other),
    }
}

fn normalize_content(content: &Value) -> Result<Value, VoyageMultimodalError> {
    if is_falsey(content) {
        return Ok(Value::Array(Vec::new()));
    }
    let Value::Array(items) = content else {
        return Err(VoyageMultimodalError::ContentNotList);
    };
    let normalized = items
        .iter()
        .map(normalize_content_item)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(normalized))
}

fn normalize_content_item(item: &Value) -> Result<Value, VoyageMultimodalError> {
    let Value::Object(fields) = item else {
        return Err(VoyageMultimodalError::ContentItemNotObject);
    };
    if fields.get("type").and_then(Value::as_str) != Some("image_url") {
        return Ok(item.clone());
    }
    let mut image_url = fields.get("image_url");
    if let Some(Value::Object(nested)) = image_url {
        image_url = nested.get("url");
    }
    let Some(image_url) = image_url.filter(|value| !value.is_null()) else {
        return Err(VoyageMultimodalError::MissingImageUrl);
    };
    if let Value::String(text) = image_url
        && text.starts_with("data:image/")
    {
        let encoded = text.split_once(',').map_or("", |(_, tail)| tail);
        return Ok(Value::Object(Map::from_iter([
            ("type".to_owned(), Value::String("image_base64".to_owned())),
            ("image_base64".to_owned(), Value::String(encoded.to_owned())),
        ])));
    }
    Ok(Value::Object(Map::from_iter([
        ("type".to_owned(), Value::String("image_url".to_owned())),
        ("image_url".to_owned(), image_url.clone()),
    ])))
}

fn is_falsey(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::Bool(true) => false,
        Value::Number(number) => number.as_f64().is_some_and(|float| float == 0.0_f64),
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(fields) => fields.is_empty(),
    }
}

#[path = "voyage_multimodal/tests.rs"]
mod tests;
