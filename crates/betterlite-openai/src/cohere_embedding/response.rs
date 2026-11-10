use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;
use serde::Serialize;

use super::request::CohereEmbeddingRequest;
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

pub struct DecodeInput<'request> {
    pub body: &'request [u8],
    pub model: &'request str,
    pub input: &'request [String],
    pub api_key: Option<&'request SecretString>,
    pub request_data: &'request CohereEmbeddingRequest,
    pub encoding: &'request dyn TextEncoding,
}

// decode_cohere_embeddings: parse, record post_call, then flatten the groups in answer order
pub fn decode_cohere_embeddings(
    input: DecodeInput<'_>,
    logging: &mut dyn EmbeddingLogging,
) -> Result<CohereEmbeddingResponse, CohereEmbeddingError> {
    let answer: Json = serde_json::from_slice(input.body)
        .map_err(|source| CohereEmbeddingError::Json { source })?;
    logging.post_call(PostCall {
        input: input.input,
        api_key: input.api_key,
        complete_input_dict: input.request_data,
        original_response: &answer,
    });
    let Json::Object(fields) = &answer else {
        return Err(CohereEmbeddingError::Field {
            name: "embeddings",
            problem: "is missing",
        });
    };
    let Some(Json::Object(groups)) = fields.get("embeddings") else {
        return Err(CohereEmbeddingError::Field {
            name: "embeddings",
            problem: "is missing",
        });
    };
    let mut data = Vec::new();
    for group in groups.values() {
        let Json::List(rows) = group else {
            return Err(CohereEmbeddingError::Field {
                name: "embeddings",
                problem: "is not a list of embeddings",
            });
        };
        data.extend(
            rows.iter()
                .enumerate()
                .map(|(index, embedding)| CohereEmbeddingRecord {
                    object: String::from("embedding"),
                    index,
                    embedding: embedding.clone(),
                }),
        );
    }
    // LiteLLM counts every input once before it reads the billed units, and that pass can fail alone
    for text in input.input {
        input
            .encoding
            .count_tokens(text)
            .map_err(|source| CohereEmbeddingError::Encoding { source })?;
    }
    let usage = calculate_usage(input.input, input.encoding, fields.get("meta"))?;
    Ok(CohereEmbeddingResponse {
        object: String::from("list"),
        data,
        model: input.model.to_owned(),
        usage,
    })
}

pub struct TransformInput<'request> {
    pub body: &'request [u8],
    pub model: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub request_data: &'request CohereEmbeddingRequest,
    pub encoding: &'request dyn TextEncoding,
}

// transform_embedding_response reads the request input from the logging details, as the pinned method does
pub fn transform_embedding_response(
    input: TransformInput<'_>,
    logging: &mut dyn EmbeddingLogging,
) -> Result<CohereEmbeddingResponse, CohereEmbeddingError> {
    let recorded = logging.model_call_details().input.clone();
    let decoded = DecodeInput {
        body: input.body,
        model: input.model,
        input: &recorded,
        api_key: input.api_key,
        request_data: input.request_data,
        encoding: input.encoding,
    };
    decode_cohere_embeddings(decoded, logging)
}
