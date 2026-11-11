use std::collections::BTreeMap;
use std::str;

use async_trait::async_trait;
use betterlite_openai::embedding::{
    EmbeddingRecord, EmbeddingResponse, EmbeddingUsage, EmbeddingVector,
};
use betterlite_prompts::arguments::json::Json;
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Number;

use super::error::OllamaGenerateError;
use crate::ollama_chat::error::OllamaError;

const DIMENSIONS: &str = "dimensions";
const EMBED_PATH: &str = "/api/embed";
const EMBEDDING_OBJECT: &str = "embedding";
const KEEP_ALIVE: &str = "keep_alive";
const LIST_OBJECT: &str = "list";
const OLLAMA_PREFIX: &str = "ollama/";
const OPTIONS: &str = "options";
const TRUNCATE: &str = "truncate";

#[async_trait]
pub trait TokenEncoder: Send + Sync {
    async fn encode(&self, text: &str) -> Result<Vec<u32>, CountError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaEmbeddingRequest {
    pub model: String,
    pub input: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncate: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<IndexMap<String, Json>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<Json>,
}

#[derive(Debug, Deserialize)]
struct OllamaEmbeddingReply {
    embeddings: Vec<Vec<Number>>,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
}

#[must_use]
pub fn embed_url(api_base: &str) -> String {
    if api_base.ends_with(EMBED_PATH) {
        api_base.to_owned()
    } else {
        format!("{api_base}{EMBED_PATH}")
    }
}

// every option outside the four body-level ones merges into options, and an options object merges too
pub fn transform_embedding_request(
    model: &str,
    prompts: &[String],
    optional_params: &IndexMap<String, Json>,
) -> Result<OllamaEmbeddingRequest, OllamaGenerateError> {
    let mut options: Option<IndexMap<String, Json>> = None;
    let mut truncate = None;
    let mut keep_alive = None;
    let mut dimensions = None;
    for (name, value) in optional_params {
        match name.as_str() {
            TRUNCATE => truncate = Some(value.clone()),
            KEEP_ALIVE => keep_alive = Some(value.clone()),
            DIMENSIONS => dimensions = Some(value.clone()),
            OPTIONS => match value {
                Json::Null => {}
                Json::Object(fields) => options = Some(fields.clone()),
                Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    return Err(OllamaGenerateError::from(OllamaError::Param {
                        field: OPTIONS,
                    }));
                }
            },
            _ => {
                options
                    .get_or_insert_with(IndexMap::new)
                    .insert(name.clone(), value.clone());
            }
        }
    }
    Ok(OllamaEmbeddingRequest {
        model: model.to_owned(),
        input: prompts.to_vec(),
        truncate,
        options,
        keep_alive,
        dimensions,
    })
}

pub async fn transform_embedding_response(
    status: u16,
    body: &[u8],
    model: &str,
    prompts: &[String],
    encoder: Option<&dyn TokenEncoder>,
) -> Result<EmbeddingResponse, OllamaGenerateError> {
    if !(200..300).contains(&status) {
        return Err(OllamaGenerateError::from(OllamaError::Api {
            status_code: status,
            message: body_message(body),
            headers: BTreeMap::new(),
        }));
    }
    let reply: OllamaEmbeddingReply = serde_json::from_slice(body)
        .map_err(|source| OllamaGenerateError::from(OllamaError::Shape { source }))?;
    let mut data = Vec::with_capacity(reply.embeddings.len());
    for (position, embedding) in reply.embeddings.into_iter().enumerate() {
        let index = u32::try_from(position).map_err(|_error| {
            OllamaGenerateError::from(OllamaError::ReplyField {
                field: "embeddings",
            })
        })?;
        data.push(EmbeddingRecord {
            object: String::from(EMBEDDING_OBJECT),
            embedding: EmbeddingVector::Floats(embedding),
            index,
        });
    }
    let prompt_tokens = match reply.prompt_eval_count {
        Some(count) => count,
        None => estimated_prompt_tokens(prompts, encoder).await?,
    };
    Ok(EmbeddingResponse {
        object: String::from(LIST_OBJECT),
        data,
        model: format!("{OLLAMA_PREFIX}{model}"),
        usage: EmbeddingUsage {
            prompt_tokens,
            completion_tokens: 0,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: prompt_tokens,
        },
    })
}

fn body_message(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_error| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

async fn estimated_prompt_tokens(
    prompts: &[String],
    encoder: Option<&dyn TokenEncoder>,
) -> Result<u32, OllamaGenerateError> {
    let Some(words) = encoder else {
        tracing::warn!(
            "the Ollama embedding reply has no prompt_eval_count and no token encoder is set, so it counts as 0 prompt tokens"
        );
        return Ok(0);
    };
    tracing::debug!(
        "the Ollama embedding reply has no prompt_eval_count, so the token encoder estimates it"
    );
    let tokens = words.encode(&prompts.join("")).await?;
    u32::try_from(tokens.len()).map_err(|_error| {
        OllamaGenerateError::from(OllamaError::ReplyField {
            field: "prompt_tokens",
        })
    })
}
