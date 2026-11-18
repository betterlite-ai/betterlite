use betterlite_openai::cohere_embedding::CohereEmbeddingError;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::cohere_legacy_embedding::decode::populate_embedding_response;
use betterlite_openai::cohere_legacy_embedding::error::CohereLegacyEmbeddingError;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingVector};
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;
use thiserror::Error;

use crate::titan_embeddings::cohere as bedrock_cohere;

pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["encoding_format", "dimensions", "input_type"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CohereRequestError {
    #[error("Input must be a list of strings")]
    InputNotText,
}

#[derive(Debug, Error)]
pub enum CohereReplyError {
    #[error("the Cohere reply body is not JSON")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Cohere reply body does not have the declared shape")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Cohere request data is not an embedding input")]
    Request {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Legacy(#[from] CohereLegacyEmbeddingError),
}

// the declared reply: an object with the fields populate reads, or a value that cannot carry any of them
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CohereReplyBody {
    // boxed: the envelope dwarfs the scalar shapes, which share this enum
    Envelope(Box<CohereReplyEnvelope>),
    Scalar(CohereReplyScalar),
}

#[derive(Debug, Deserialize)]
struct CohereReplyEnvelope {
    // boxed: the grouped rows map dwarfs every other reply shape
    embeddings: Option<Box<ReplyEmbeddings>>,
    response_type: Option<String>,
    meta: Option<CohereReplyMeta>,
}

// CohereEmbeddingResponse declares number rows; a reply carries them alone or grouped by type, as base64 strings too
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ReplyEmbeddings {
    Rows(Vec<EmbeddingVector>),
    Groups(IndexMap<String, Vec<EmbeddingVector>>),
}

#[derive(Debug, Deserialize)]
struct CohereReplyMeta {
    billed_units: Option<CohereBilledUnits>,
}

// Meta's billed units are the two integer counts calculate_usage reads
#[derive(Debug, Deserialize)]
struct CohereBilledUnits {
    input_tokens: Option<i64>,
    images: Option<i64>,
}

// a reply that is not an object: populate refuses it before reading any field, so these values only fix the shape
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CohereReplyScalar {
    Bool(bool),
    Number(Number),
    Text(String),
    List(Vec<EmbeddingVector>),
    Null,
}

// the request fields the reply input falls back to, typed as the Cohere request writes them
#[derive(Debug, Deserialize)]
struct CohereRequestBody {
    texts: Option<Vec<String>>,
    images: Option<Vec<String>>,
}

impl CohereReplyBody {
    // B1418's populate takes JSON, so the typed reply turns back into JSON only here
    fn into_json(self) -> Json {
        match self {
            Self::Envelope(envelope) => envelope_json(*envelope),
            Self::Scalar(CohereReplyScalar::Bool(value)) => Json::Bool(value),
            Self::Scalar(CohereReplyScalar::Number(number)) => Json::Number(number),
            Self::Scalar(CohereReplyScalar::Text(text)) => Json::Text(text),
            Self::Scalar(CohereReplyScalar::List(rows)) => {
                Json::List(rows.into_iter().map(row_json).collect())
            }
            Self::Scalar(CohereReplyScalar::Null) => Json::Null,
        }
    }
}

fn envelope_json(envelope: CohereReplyEnvelope) -> Json {
    let mut fields = Object::new();
    if let Some(embeddings) = envelope.embeddings {
        let rows = match *embeddings {
            ReplyEmbeddings::Rows(rows) => Json::List(rows.into_iter().map(row_json).collect()),
            ReplyEmbeddings::Groups(groups) => Json::Object(
                groups
                    .into_iter()
                    .map(|(name, rows)| {
                        (name, Json::List(rows.into_iter().map(row_json).collect()))
                    })
                    .collect(),
            ),
        };
        fields.insert(String::from("embeddings"), rows);
    }
    if let Some(kind) = envelope.response_type {
        fields.insert(String::from("response_type"), Json::Text(kind));
    }
    if let Some(meta) = envelope.meta {
        let mut meta_fields = Object::new();
        if let Some(billed) = meta.billed_units {
            let mut counts = Object::new();
            if let Some(tokens) = billed.input_tokens {
                counts.insert(
                    String::from("input_tokens"),
                    Json::Number(Number::from(tokens)),
                );
            }
            if let Some(images) = billed.images {
                counts.insert(String::from("images"), Json::Number(Number::from(images)));
            }
            meta_fields.insert(String::from("billed_units"), Json::Object(counts));
        }
        fields.insert(String::from("meta"), Json::Object(meta_fields));
    }
    Json::Object(fields)
}

fn row_json(embedding: EmbeddingVector) -> Json {
    match embedding {
        EmbeddingVector::Base64(text) => Json::Text(text),
        EmbeddingVector::Floats(numbers) => {
            Json::List(numbers.into_iter().map(Json::Number).collect())
        }
    }
}

// map_openai_params: the Bedrock mapper runs first, then input_type is copied over unchanged
#[must_use]
pub fn map_openai_params(non_default_params: &Object, optional_params: Object) -> Object {
    let mut mapped = bedrock_cohere::map_openai_params(non_default_params, optional_params);
    if let Some(input_type) = non_default_params.get("input_type") {
        mapped.insert(String::from("input_type"), input_type.clone());
    }
    mapped
}

// transform_embedding_request: only a text or a list of texts reaches the Cohere body
pub fn transform_request(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Result<Object, CohereRequestError> {
    let texts = match input {
        EmbeddingInput::Text(text) => vec![text.clone()],
        EmbeddingInput::Texts(texts) => texts.clone(),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            return Err(CohereRequestError::InputNotText);
        }
    };
    Ok(bedrock_cohere::transform_request(
        model,
        &texts,
        optional_params,
    ))
}

// transform_embedding_response: the logged input is preferred over the body's texts and images
pub fn transform_response(
    body: &[u8],
    model: &str,
    logged_input: Option<&EmbeddingInput>,
    request_data: &Object,
    encoding: &dyn TextEncoding,
) -> Result<LegacyEmbeddingResponse, CohereReplyError> {
    let reply: CohereReplyBody = serde_json::from_slice(body).map_err(|source| {
        if source.is_data() {
            CohereReplyError::Shape { source }
        } else {
            CohereReplyError::Body { source }
        }
    })?;
    let input = reply_input(logged_input, request_data)?;
    populate_embedding_response(&reply.into_json(), model, &input, encoding)
        .map_err(CohereReplyError::Legacy)
}

fn reply_input(
    logged_input: Option<&EmbeddingInput>,
    request_data: &Object,
) -> Result<Vec<String>, CohereReplyError> {
    if let Some(input) = logged_input.filter(|input| has_value(input)) {
        return match input {
            EmbeddingInput::Text(text) => Ok(vec![text.clone()]),
            EmbeddingInput::Texts(texts) => Ok(texts.clone()),
            EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
                Err(CohereReplyError::Legacy(CohereLegacyEmbeddingError::Usage(
                    CohereEmbeddingError::InputNotText,
                )))
            }
        };
    }
    let value = serde_json::to_value(request_data)
        .map_err(|source| CohereReplyError::Request { source })?;
    let body: CohereRequestBody =
        serde_json::from_value(value).map_err(|source| CohereReplyError::Request { source })?;
    Ok([body.texts, body.images]
        .into_iter()
        .flatten()
        .find(|texts| !texts.is_empty())
        .unwrap_or_default())
}

// Cohere's input choice skips an empty value, as the pinned code's `or` chain does
const fn has_value(input: &EmbeddingInput) -> bool {
    match input {
        EmbeddingInput::Text(text) => !text.is_empty(),
        EmbeddingInput::Texts(texts) => !texts.is_empty(),
        EmbeddingInput::Tokens(tokens) => !tokens.is_empty(),
        EmbeddingInput::TokenLists(lists) => !lists.is_empty(),
    }
}
mod tests;
