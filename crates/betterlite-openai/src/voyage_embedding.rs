use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoyageEmbeddingVariant {
    Text,
    Contextual,
}

#[derive(Debug, Error)]
pub enum VoyageEmbeddingError {
    #[error("Voyage embedding response was not valid JSON: {message}")]
    Decode {
        status: u16,
        url: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoyageEmbeddingConfig {
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoyageContextConfig {
    pub api_base: Option<String>,
}

pub fn endpoint(base: Option<&str>, variant: VoyageEmbeddingVariant) -> String {
    let suffix = match variant {
        VoyageEmbeddingVariant::Text => "/embeddings",
        VoyageEmbeddingVariant::Contextual => "/contextualizedembeddings",
    };
    match base {
        Some(base) if !base.is_empty() => {
            if base.ends_with(suffix) {
                base.to_owned()
            } else {
                format!("{base}{suffix}")
            }
        }
        _ => match variant {
            VoyageEmbeddingVariant::Text => "https://api.voyageai.com/v1/embeddings".to_owned(),
            VoyageEmbeddingVariant::Contextual => {
                "https://api.voyageai.com/v1/contextualizedembeddings".to_owned()
            }
        },
    }
}

pub fn request(
    model: &str,
    input: Value,
    options: &Map<String, Value>,
    variant: VoyageEmbeddingVariant,
) -> Value {
    let mut result = Map::new();
    let input = if variant == VoyageEmbeddingVariant::Contextual && input.is_string() {
        Value::Array(vec![input])
    } else {
        input
    };
    result.insert(
        if variant == VoyageEmbeddingVariant::Text {
            "input"
        } else {
            "inputs"
        }
        .to_owned(),
        input.clone(),
    );
    result.insert("model".to_owned(), Value::String(model.to_owned()));
    if variant == VoyageEmbeddingVariant::Contextual
        && is_flat(&input)
        && options.get("input_type") != Some(&Value::String("query".to_owned()))
    {
        result.insert("enable_auto_chunking".to_owned(), Value::Bool(true));
        result.insert("chunk_size".to_owned(), Value::from(32000));
        result.insert(
            "input_type".to_owned(),
            Value::String("document".to_owned()),
        );
    }
    for (key, value) in options {
        result.insert(
            match key.as_str() {
                "dimensions" => "output_dimension".to_owned(),
                other => other.to_owned(),
            },
            value.clone(),
        );
    }
    Value::Object(result)
}

fn is_flat(input: &Value) -> bool {
    match input {
        Value::String(_) => true,
        Value::Array(items) => items.iter().all(Value::is_string),
        _ => false,
    }
}

pub fn voyage_headers<F>(
    headers: Map<String, Value>,
    api_key: Option<&str>,
    lookup: F,
) -> Map<String, Value>
where
    F: Fn(&str) -> Option<String>,
{
    let mut replacement = Map::new();
    replacement.insert(
        "Authorization".to_owned(),
        Value::String(voyage_credentials(api_key, lookup)),
    );
    drop(headers);
    replacement
}

pub fn voyage_credentials<F>(api_key: Option<&str>, lookup: F) -> String
where
    F: Fn(&str) -> Option<String>,
{
    match api_key {
        Some(key) => format!("Bearer {key}"),
        None => format!(
            "Bearer {}",
            lookup("VOYAGE_API_KEY")
                .filter(|s| !s.is_empty())
                .or_else(|| lookup("VOYAGE_AI_API_KEY").filter(|s| !s.is_empty()))
                .or_else(|| lookup("VOYAGE_AI_TOKEN").filter(|s| !s.is_empty()))
                .unwrap_or_else(|| "None".to_owned())
        ),
    }
}

pub fn decode_voyage_embedding_reply(
    bytes: &[u8],
    status: u16,
    variant: VoyageEmbeddingVariant,
) -> Result<Value, VoyageEmbeddingError> {
    let text = String::from_utf8_lossy(bytes);
    let parsed: Value =
        serde_json::from_slice(bytes).map_err(|error| VoyageEmbeddingError::Decode {
            status,
            url: match variant {
                VoyageEmbeddingVariant::Text => "https://api.voyageai.com/v1/embeddings",
                VoyageEmbeddingVariant::Contextual => {
                    "https://api.voyageai.com/v1/contextualizedembeddings"
                }
            }
            .to_owned(),
            message: if text.is_empty() {
                error.to_string()
            } else {
                text.into_owned()
            },
        })?;
    let usage = parsed
        .get("usage")
        .and_then(|value| value.get("total_tokens"))
        .cloned()
        .unwrap_or(Value::from(0));
    Ok(
        serde_json::json!({"model": parsed.get("model").cloned().unwrap_or(Value::Null), "data": parsed.get("data").cloned().unwrap_or(Value::Null), "object": parsed.get("object").cloned().unwrap_or(Value::Null), "usage": {"prompt_tokens": usage, "total_tokens": usage}}),
    )
}

#[path = "voyage_embedding/tests.rs"]
mod tests;
