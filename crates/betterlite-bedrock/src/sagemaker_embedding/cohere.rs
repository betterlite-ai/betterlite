use betterlite_openai::cohere_embedding::CohereEmbeddingError;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::cohere_legacy_embedding::decode::populate_embedding_response;
use betterlite_openai::cohere_legacy_embedding::error::CohereLegacyEmbeddingError;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Deserialize;
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
    #[error("the Cohere request data is not an embedding input")]
    Request {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Legacy(#[from] CohereLegacyEmbeddingError),
}

// B1418's populate reads the reply, so this checks the body's shape and hands its values back unchanged
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CohereReplyBody {
    Envelope(CohereReplyEnvelope),
    Other(Json),
}

#[derive(Debug, Deserialize)]
struct CohereReplyEnvelope {
    embeddings: Option<ReplyEmbeddings>, // its rows are copied verbatim into the records
    response_type: Option<Json>,         // populate compares only the embeddings_by_type marker
    meta: Option<Json>,                  // calculate_usage reads billed_units from it
    #[serde(flatten)]
    rest: Object,   // every other key is populate's to ignore
}

// the two row shapes populate reads; any other value reaches it untouched and it answers Embeddings
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ReplyEmbeddings {
    Rows(Vec<Json>),
    Groups(IndexMap<String, Vec<Json>>),
    Other(Json),
}

// the request fields the reply input falls back to, checked as an embedding input only once chosen
#[derive(Debug, Deserialize)]
struct CohereRequestBody {
    texts: Option<Json>,
    images: Option<Json>,
}

impl CohereReplyBody {
    fn into_json(self) -> Json {
        match self {
            Self::Envelope(envelope) => envelope.into_json(),
            Self::Other(value) => value,
        }
    }
}

impl CohereReplyEnvelope {
    fn into_json(self) -> Json {
        let mut fields = self.rest;
        if let Some(embeddings) = self.embeddings {
            fields.insert(String::from("embeddings"), embeddings.into_json());
        }
        if let Some(kind) = self.response_type {
            fields.insert(String::from("response_type"), kind);
        }
        if let Some(meta) = self.meta {
            fields.insert(String::from("meta"), meta);
        }
        Json::Object(fields)
    }
}

impl ReplyEmbeddings {
    fn into_json(self) -> Json {
        match self {
            Self::Rows(rows) => Json::List(rows),
            Self::Groups(groups) => Json::Object(
                groups
                    .into_iter()
                    .map(|(name, rows)| (name, Json::List(rows)))
                    .collect(),
            ),
            Self::Other(value) => value,
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
    let reply: CohereReplyBody =
        serde_json::from_slice(body).map_err(|source| CohereReplyError::Body { source })?;
    let input = reply_input(logged_input, request_data)?;
    populate_embedding_response(&reply.into_json(), model, &input, encoding)
        .map_err(CohereReplyError::Legacy)
}

fn reply_input(
    logged_input: Option<&EmbeddingInput>,
    request_data: &Object,
) -> Result<Vec<String>, CohereReplyError> {
    let mut chosen = logged_input.filter(|input| has_value(input)).cloned();
    if chosen.is_none() {
        let value = serde_json::to_value(request_data)
            .map_err(|source| CohereReplyError::Request { source })?;
        let body: CohereRequestBody =
            serde_json::from_value(value).map_err(|source| CohereReplyError::Request { source })?;
        for candidate in [body.texts, body.images].into_iter().flatten() {
            if !candidate.is_truthy() {
                continue;
            }
            let encoded = serde_json::to_value(candidate)
                .map_err(|source| CohereReplyError::Request { source })?;
            chosen = Some(
                serde_json::from_value(encoded)
                    .map_err(|source| CohereReplyError::Request { source })?,
            );
            break;
        }
    }
    match chosen {
        None => Ok(Vec::new()),
        Some(EmbeddingInput::Text(text)) => Ok(vec![text]),
        Some(EmbeddingInput::Texts(texts)) => Ok(texts),
        Some(EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_)) => {
            Err(CohereReplyError::Legacy(CohereLegacyEmbeddingError::Usage(
                CohereEmbeddingError::InputNotText,
            )))
        }
    }
}

// Cohere's input choice skips an empty value, as the pinned code's `or` chain does
fn has_value(input: &EmbeddingInput) -> bool {
    match input {
        EmbeddingInput::Text(text) => !text.is_empty(),
        EmbeddingInput::Texts(texts) => !texts.is_empty(),
        EmbeddingInput::Tokens(tokens) => !tokens.is_empty(),
        EmbeddingInput::TokenLists(lists) => !lists.is_empty(),
    }
}
mod tests;
