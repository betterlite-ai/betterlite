use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_openai::embedding::reply_error::reply_error;
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingUsage};
use serde::Deserialize;

use crate::embeddings::error::FoundryEmbeddingError;

// A performed call's reply; each absent field leaves the response as it was, a null model clears it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingReply {
    pub object: Option<String>,
    pub model: Option<Option<String>>,
    pub data: Option<Option<Vec<EmbeddingRecord>>>,
    pub usage: Option<EmbeddingUsage>,
    pub additional_headers: AdditionalHeaders,
}

// B004's embedding response shape: a fresh object, data set later, usage counting from zero
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundryEmbeddingResponse {
    pub object: String,
    pub data: Vec<EmbeddingRecord>,
    pub model: Option<String>,
    pub usage: EmbeddingUsage,
    pub additional_headers: AdditionalHeaders,
}

#[derive(Debug, Deserialize)]
struct RawReply {
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    object: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    model: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    data: Option<Option<Vec<EmbeddingRecord>>>,
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    usage: Option<Option<RawUsage>>,
}

#[derive(Debug, Deserialize)]
struct RawUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

impl Default for FoundryEmbeddingResponse {
    fn default() -> Self {
        Self {
            object: String::from("list"),
            data: Vec::new(),
            model: None,
            usage: EmbeddingUsage {
                prompt_tokens: 0,
                completion_tokens: 0,
                completion_tokens_details: None,
                prompt_tokens_details: None,
                total_tokens: 0,
            },
            additional_headers: AdditionalHeaders::default(),
        }
    }
}

impl FoundryEmbeddingResponse {
    pub fn apply_reply(&mut self, reply: EmbeddingReply) {
        if let Some(object) = reply.object {
            self.object = object;
        }
        if let Some(model) = reply.model {
            self.model = model;
        }
        // LiteLLM copies only the three counts; the details on the response stay as they were
        if let Some(usage) = reply.usage {
            self.usage.completion_tokens = usage.completion_tokens;
            self.usage.prompt_tokens = usage.prompt_tokens;
            self.usage.total_tokens = usage.total_tokens;
        }
        self.additional_headers = reply.additional_headers;
    }
}

pub fn decode_foundry_embedding(body: &[u8]) -> Result<EmbeddingReply, FoundryEmbeddingError> {
    let value =
        serde_json::from_slice(body).map_err(|source| FoundryEmbeddingError::Json { source })?;
    if let Some(failure) = reply_error(&value) {
        return Err(FoundryEmbeddingError::ReplyError {
            status: failure.status,
        });
    }
    let raw: RawReply =
        serde_json::from_value(value).map_err(|source| FoundryEmbeddingError::Json { source })?;
    if matches!(raw.object, Some(None)) {
        return Err(FoundryEmbeddingError::FieldType {
            field: "object",
            expected: "a string",
        });
    }
    Ok(EmbeddingReply {
        object: raw.object.flatten(),
        model: raw.model,
        data: raw.data,
        usage: raw.usage.flatten().map(|usage| EmbeddingUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: usage.total_tokens,
        }),
        additional_headers: AdditionalHeaders::default(),
    })
}

// LiteLLM fills the answer by original input position, taking each performed call's entries in order
pub fn interleave(
    input_len: usize,
    image_indices: &[usize],
    image: Option<&[EmbeddingRecord]>,
    text: Option<&[EmbeddingRecord]>,
) -> Result<Vec<EmbeddingRecord>, FoundryEmbeddingError> {
    match (image, text) {
        (Some(images), Some(texts)) => combined(input_len, image_indices, images, texts),
        (image_records, text_records) => {
            Ok(image_records.or(text_records).unwrap_or_default().to_vec())
        }
    }
}

fn combined(
    input_len: usize,
    image_indices: &[usize],
    images: &[EmbeddingRecord],
    texts: &[EmbeddingRecord],
) -> Result<Vec<EmbeddingRecord>, FoundryEmbeddingError> {
    let mut images = images.iter();
    let mut texts = texts.iter();
    let mut combined = Vec::with_capacity(input_len);
    for index in 0..input_len {
        let chosen = if image_indices.contains(&index) {
            images.next().ok_or(FoundryEmbeddingError::TooFewEntries {
                entries: "image embeddings",
            })?
        } else {
            texts.next().ok_or(FoundryEmbeddingError::TooFewEntries {
                entries: "text embeddings",
            })?
        };
        combined.push(chosen.clone());
    }
    Ok(combined)
}
