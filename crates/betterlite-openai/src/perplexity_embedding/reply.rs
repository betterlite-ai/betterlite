use std::ops::Div as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_json::error::Category;

use super::error::PerplexityEmbeddingError;
use super::get_error_class;
use crate::embedding::EmbeddingUsage;
use crate::openai_base_config::Headers;

// a 2560-dimension float vector per input stays far below this, and the cap bounds the copy
pub const MAX_REPLY_BYTES: usize = 0x400_0000;
const INT8_SCALE: f64 = 127.0_f64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PerplexityEmbeddingReply {
    pub model: String,
    pub object: String,
    // LiteLLM copies every key of a data item and rewrites only its embedding
    pub data: Vec<Object>,
    pub usage: EmbeddingUsage,
}

#[derive(Deserialize)]
struct WireReply {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    data: Option<Vec<Object>>,
    #[serde(default)]
    usage: Option<WireUsage>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    total_tokens: Option<u32>,
    // a flattened map keeps serde from accepting a JSON array in place of this object
    #[serde(flatten)]
    _extra: Object,
}

pub fn transform_embedding_response(
    model: &str,
    status_code: u16,
    raw: &[u8],
) -> Result<PerplexityEmbeddingReply, PerplexityEmbeddingError> {
    if raw.len() > MAX_REPLY_BYTES {
        return Err(PerplexityEmbeddingError::TooLarge {
            limit: MAX_REPLY_BYTES,
        });
    }
    let wire = decode_wire(status_code, raw)?;
    let mut data = wire.data.unwrap_or_default();
    for item in &mut data {
        let decoded = decoded_embedding(item.get("embedding"))?;
        item.insert(String::from("embedding"), decoded);
    }
    let (prompt_tokens, total_tokens) = wire.usage.map_or((None, None), |usage| {
        (usage.prompt_tokens, usage.total_tokens)
    });
    let total = total_tokens.unwrap_or(0);
    Ok(PerplexityEmbeddingReply {
        model: wire.model.unwrap_or_else(|| model.to_owned()),
        object: wire.object.unwrap_or_else(|| String::from("list")),
        data,
        usage: EmbeddingUsage {
            // LiteLLM's `prompt_tokens or total_tokens` falls back on 0 as well as on a missing count
            prompt_tokens: prompt_tokens.filter(|count| *count != 0).unwrap_or(total),
            completion_tokens: 0,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: total,
        },
    })
}

// a reply that is not JSON raises through the Perplexity error class, and one of the wrong shape is a Shape error
fn decode_wire(status_code: u16, raw: &[u8]) -> Result<WireReply, PerplexityEmbeddingError> {
    match serde_json::from_slice(raw) {
        Ok(wire) => Ok(wire),
        Err(source) if source.classify() == Category::Data => {
            Err(PerplexityEmbeddingError::Shape { source })
        }
        Err(_not_json) => {
            // httpx reads raw_response.text with errors="replace", so each invalid sequence reads as one U+FFFD
            let mut text = String::with_capacity(raw.len());
            for chunk in raw.utf8_chunks() {
                text.push_str(chunk.valid());
                if !chunk.invalid().is_empty() {
                    text.push(char::REPLACEMENT_CHARACTER);
                }
            }
            Err(PerplexityEmbeddingError::Provider {
                error: Box::new(get_error_class(&text, status_code, &Headers::new())),
            })
        }
    }
}

// a string is base64 of signed bytes, each divided by 127; a list or any other value stays as sent
fn decoded_embedding(embedding: Option<&Json>) -> Result<Json, PerplexityEmbeddingError> {
    let Some(value) = embedding else {
        return Ok(Json::Null);
    };
    let Json::Text(encoded) = value else {
        return Ok(value.clone());
    };
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|source| PerplexityEmbeddingError::Base64 { source })?;
    // struct.unpack(f"{count}b") reads a signed byte; Div::div replaces the banned / operator
    let floats = bytes
        .into_iter()
        .map(|byte| f64::from(byte.cast_signed()).div(INT8_SCALE))
        .filter_map(Number::from_f64)
        .map(Json::Number)
        .collect();
    Ok(Json::List(floats))
}
