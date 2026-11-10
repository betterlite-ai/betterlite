use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::usage::{CohereEmbeddingUsage, calculate_usage};
use super::{CohereEmbeddingError, EmbeddingLogging, PostCall, TextEncoding};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CohereEmbeddingResponse {
    pub object: String,
    pub data: Vec<CohereEmbeddingRecord>,
    pub model: String,
    pub usage: CohereEmbeddingUsage,
}

// the v2 output omits the embedding type, unlike the legacy grouped rows
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CohereEmbeddingRecord {
    pub object: String,
    pub index: usize,
    pub embedding: Json,
}

#[derive(Deserialize)]
struct Answer {
    embeddings: IndexMap<String, Vec<Json>>,
    meta: Option<Json>,
}

#[derive(Clone, Copy)]
pub struct DecodeInput<'request> {
    pub body: &'request [u8],
    pub model: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub request_data: &'request Object,
    pub encoding: &'request dyn TextEncoding,
}

// the request input comes from the logging details, as in the pinned method
pub fn transform_embedding_response(
    reply: DecodeInput<'_>,
    logging: &mut dyn EmbeddingLogging,
) -> Result<CohereEmbeddingResponse, CohereEmbeddingError> {
    let input = logging.input().to_vec();
    let original: Json = serde_json::from_slice(reply.body)?;
    logging.post_call(PostCall {
        input: &input,
        api_key: reply.api_key,
        complete_input_dict: reply.request_data,
        original_response: &original,
    });
    let answer: Answer = serde_json::from_slice(reply.body)?;
    let data = answer
        .embeddings
        .into_values()
        .flat_map(|rows| rows.into_iter().enumerate())
        .map(|(index, embedding)| CohereEmbeddingRecord {
            object: String::from("embedding"),
            index,
            embedding,
        })
        .collect();
    // LiteLLM counts every input once before it reads the billed units, and that pass can fail alone
    for text in &input {
        reply.encoding.count_tokens(text)?;
    }
    Ok(CohereEmbeddingResponse {
        object: String::from("list"),
        data,
        model: reply.model.to_owned(),
        usage: calculate_usage(&input, reply.encoding, answer.meta.as_ref())?,
    })
}
