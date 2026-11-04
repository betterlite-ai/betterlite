use std::str;

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
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoyageEmbeddingConfig {
    pub api_base: Option<String>,
}

// The short Rust name maps to LiteLLM's VoyageContextualEmbeddingConfig: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/voyage/embedding/transformation_contextual.py#L40
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoyageContextConfig {
    pub api_base: Option<String>,
}

#[must_use]
pub fn endpoint(base: Option<&str>, variant: VoyageEmbeddingVariant) -> String {
    let suffix = match variant {
        VoyageEmbeddingVariant::Text => "/embeddings",
        VoyageEmbeddingVariant::Contextual => "/contextualizedembeddings",
    };
    match base {
        Some(supplied) if !supplied.is_empty() => {
            if supplied.ends_with(suffix) {
                supplied.to_owned()
            } else {
                format!("{supplied}{suffix}")
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

#[must_use]
pub fn request(
    model: &str,
    input: Value,
    options: &Map<String, Value>,
    variant: VoyageEmbeddingVariant,
) -> Value {
    let mut result = Map::new();
    let normalized = if variant == VoyageEmbeddingVariant::Contextual && input.is_string() {
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
        normalized.clone(),
    );
    result.insert("model".to_owned(), Value::String(model.to_owned()));
    if variant == VoyageEmbeddingVariant::Contextual
        && is_flat(&normalized)
        && options.get("input_type") != Some(&Value::String("query".to_owned()))
    {
        result.insert("enable_auto_chunking".to_owned(), Value::Bool(true));
        result.insert("chunk_size".to_owned(), Value::from(32_000_u32));
        result.insert(
            "input_type".to_owned(),
            Value::String("document".to_owned()),
        );
    }
    let mut mapped = options.clone();
    if let Some(dimensions) = mapped.remove("dimensions") {
        mapped.insert("output_dimension".to_owned(), dimensions);
    }
    result.extend(mapped);
    Value::Object(result)
}

fn is_flat(input: &Value) -> bool {
    match input {
        Value::String(_) => true,
        Value::Array(items) => items.iter().all(Value::is_string),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => false,
    }
}

#[must_use]
pub fn is_contextual_model(model: &str) -> bool {
    let lowered = model.to_lowercase();
    let chars = lowered.chars().collect::<Vec<char>>();
    for window in chars.windows("context".len()) {
        if window.iter().collect::<String>() == "context" {
            return true;
        }
    }
    false
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
    api_key.map_or_else(
        || {
            format!(
                "Bearer {}",
                lookup("VOYAGE_API_KEY")
                    .filter(|value| !value.is_empty())
                    .or_else(|| lookup("VOYAGE_AI_API_KEY").filter(|value| !value.is_empty()))
                    .or_else(|| lookup("VOYAGE_AI_TOKEN").filter(|value| !value.is_empty()))
                    .unwrap_or_else(|| "None".to_owned())
            )
        },
        |key| format!("Bearer {key}"),
    )
}

pub fn decode_voyage_embedding_reply(
    bytes: &[u8],
    status: u16,
    variant: VoyageEmbeddingVariant,
) -> Result<Value, VoyageEmbeddingError> {
    let parsed: Value =
        serde_json::from_slice(bytes).map_err(|source| VoyageEmbeddingError::Decode {
            status,
            url: match variant {
                VoyageEmbeddingVariant::Text => "https://api.voyageai.com/v1/embeddings",
                VoyageEmbeddingVariant::Contextual => {
                    "https://api.voyageai.com/v1/contextualizedembeddings"
                }
            }
            .to_owned(),
            message: str::from_utf8(bytes).map_or_else(|_| source.to_string(), str::to_owned),
            source,
        })?;
    Ok(project_response(&parsed))
}

#[must_use]
pub fn project_response(parsed: &Value) -> Value {
    let usage = parsed
        .get("usage")
        .and_then(|value| value.get("total_tokens"))
        .cloned()
        .unwrap_or_else(|| Value::from(0_u32));
    Value::Object(Map::from_iter([
        (
            "model".to_owned(),
            parsed.get("model").cloned().unwrap_or(Value::Null),
        ),
        (
            "data".to_owned(),
            parsed.get("data").cloned().unwrap_or(Value::Null),
        ),
        (
            "object".to_owned(),
            parsed.get("object").cloned().unwrap_or(Value::Null),
        ),
        (
            "usage".to_owned(),
            Value::Object(Map::from_iter([
                ("prompt_tokens".to_owned(), usage.clone()),
                ("total_tokens".to_owned(), usage),
            ])),
        ),
    ]))
}

#[path = "voyage_embedding/tests.rs"]
mod tests;
