use betterlite_models::decisions::HiddenParams;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::edenai_common::{
    CONTENT_TYPE, EdenAiEnv, JSON_CONTENT_TYPE, MissingApiKey, authorized_headers, endpoint_url,
    pick, reported_cost, set_response_cost, with_header,
};
use crate::embedding::reply_error::reply_error;
use crate::embedding::{EmbeddingInput, EmbeddingRecord, EmbeddingResponse, EmbeddingUsage};
use crate::openai_base_config::{Headers, Object};

mod tests;

// LiteLLM's _SUPPORTED_PARAMS, in the order its get_supported_openai_params returns
pub const SUPPORTED_PARAMS: [&str; 3] = ["dimensions", "encoding_format", "user"];

const EMBEDDINGS_PATH: &str = "embeddings";
// LiteLLM's text for an error object that carries no message of its own
const DEFAULT_ERROR: &str = "Error in response object";

#[derive(Debug, Error)]
pub enum EdenAiEmbeddingError {
    #[error(transparent)]
    MissingKey(#[from] MissingApiKey),
    #[error("the embedding input could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    // a meaningful error object inside a successful reply, as convert_to_model_response_object raises it
    #[error("{message}")]
    ReplyError { status_code: u16, message: String },
    #[error("the Eden AI embeddings reply is not an embeddings response: {source}")]
    Reply {
        #[source]
        source: serde_json::Error,
    },
}

impl EdenAiEmbeddingError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::MissingKey(error) => error.status_code,
            Self::ReplyError { status_code, .. } => *status_code,
            Self::Encode { .. } | Self::Reply { .. } => 500,
        }
    }
}

// the caller passes the params it already mapped; only the three Eden-supported keys survive
#[must_use]
pub fn map_openai_params(non_default_params: &Object, optional_params: &Object) -> Object {
    let mut mapped = optional_params.clone();
    mapped.extend(pick(non_default_params, &SUPPORTED_PARAMS));
    mapped
}

pub struct TransformInput<'request> {
    pub model: &'request str,
    pub input: &'request EmbeddingInput,
    pub optional_params: &'request Object,
    pub headers: &'request Headers,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub env: &'request EdenAiEnv,
}

#[derive(Debug)]
pub struct EmbeddingPlan {
    pub url: String,
    pub headers: Headers,
    pub body: Object,
}

// LiteLLM's transform_embedding_request: model and input first, then the optional params, which overwrite them
pub fn request_body(
    model: &str,
    input: &EmbeddingInput,
    optional_params: &Object,
) -> Result<Object, EdenAiEmbeddingError> {
    let embedded =
        serde_json::to_value(input).map_err(|source| EdenAiEmbeddingError::Encode { source })?;
    let mut body = Object::from_iter([
        (String::from("model"), Value::String(model.to_owned())),
        (String::from("input"), embedded),
    ]);
    body.extend(optional_params.clone());
    Ok(body)
}

// LiteLLM's json_headers and endpoint_url with the request body in one step, so the caller only sends the plan
pub fn plan_embedding(input: &TransformInput<'_>) -> Result<EmbeddingPlan, EdenAiEmbeddingError> {
    let authorized = authorized_headers(input.headers, input.api_key, input.model, input.env)?;
    let headers = with_header(&authorized, CONTENT_TYPE, JSON_CONTENT_TYPE);
    let url = endpoint_url(input.api_base, EMBEDDINGS_PATH, input.env);
    let body = request_body(input.model, input.input, input.optional_params)?;
    Ok(EmbeddingPlan { url, headers, body })
}

// the reply fields convert_to_model_response_object reads, with the defaults its EmbeddingResponse starts from
#[derive(Debug, Deserialize)]
struct WireReply {
    #[serde(default)]
    object: Option<String>,
    data: Vec<EmbeddingRecord>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
}

#[derive(Debug)]
pub struct EmbeddingReply {
    pub response: EmbeddingResponse,
    pub hidden_params: HiddenParams,
}

// the body as an embedding response with Eden's cost lifted into the hidden params; a meaningful error object is raised first
pub fn transform_response(body: &[u8]) -> Result<EmbeddingReply, EdenAiEmbeddingError> {
    let payload: Value =
        serde_json::from_slice(body).map_err(|source| EdenAiEmbeddingError::Reply { source })?;
    if let Some(failure) = reply_error(&payload) {
        return Err(EdenAiEmbeddingError::ReplyError {
            status_code: failure.status,
            message: failure
                .message
                .unwrap_or_else(|| String::from(DEFAULT_ERROR)),
        });
    }
    let cost = reported_cost(&payload);
    let reply: WireReply =
        serde_json::from_value(payload).map_err(|source| EdenAiEmbeddingError::Reply { source })?;
    let usage = reply.usage.unwrap_or_default();
    let response = EmbeddingResponse {
        object: reply.object.unwrap_or_else(|| String::from("list")),
        data: reply.data,
        model: reply.model.unwrap_or_default(),
        usage: EmbeddingUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            completion_tokens_details: None,
            prompt_tokens_details: None,
            total_tokens: usage.total_tokens,
        },
    };
    let mut hidden_params = HiddenParams::new();
    set_response_cost(&mut hidden_params, cost);
    Ok(EmbeddingReply {
        response,
        hidden_params,
    })
}
