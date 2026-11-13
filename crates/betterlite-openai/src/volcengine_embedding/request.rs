use secrecy::SecretString;
use serde_json::Value;

use crate::embedding::EmbeddingInput;
use crate::openai_base_config::{Headers, Object};
use crate::volcengine_common;

use super::error::VolcEngineEmbeddingError;

// the version segment goes on unless the base already carries it
#[must_use]
pub fn get_complete_url(api_base: Option<&str>) -> String {
    let base = volcengine_common::base_url(api_base);
    if base.ends_with("/api/v3") {
        format!("{base}/embeddings")
    } else {
        format!("{base}/api/v3/embeddings")
    }
}

// the caller's headers stay and the two Volcengine headers win on an exactly spelled key
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
) -> Result<Headers, VolcEngineEmbeddingError> {
    let key = api_key.ok_or(VolcEngineEmbeddingError::MissingApiKey)?;
    let mut merged = headers.clone();
    merged.extend(volcengine_common::headers(key, &Headers::new()));
    Ok(merged)
}

// a single string becomes a one item list, and only the two body options are added when they hold a value
#[must_use]
pub fn transform_embedding_request(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Object {
    let body_input = match input {
        EmbeddingInput::Text(text) => Value::Array(vec![Value::String(text.clone())]),
        EmbeddingInput::Texts(items) => {
            Value::Array(items.iter().cloned().map(Value::String).collect())
        }
        EmbeddingInput::Tokens(items) => {
            Value::Array(items.iter().copied().map(Value::from).collect())
        }
        EmbeddingInput::TokenLists(lists) => Value::Array(
            lists
                .iter()
                .map(|list| Value::Array(list.iter().copied().map(Value::from).collect()))
                .collect(),
        ),
    };
    let mut data = Object::from_iter([
        (String::from("model"), Value::String(model.to_owned())),
        (String::from("input"), body_input),
    ]);
    for name in ["encoding_format", "user"] {
        if let Some(value) = optional_params.get(name).filter(|value| !value.is_null()) {
            data.insert(String::from(name), value.clone());
        }
    }
    data
}
