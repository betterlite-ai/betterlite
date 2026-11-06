use std::io;
use std::time::{Duration, Instant};

use betterlite_config::sections::mock_response::MockResponse;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::resolve_mock;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::{JsonPost, UpstreamResponse};
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::chat::client_mock;
use crate::chat::deployment::{ChatDeployment, DeploymentError, Provider};
use crate::chat::execution::ChatExecutor;
use crate::chat::mock_model;

// LiteLLM's raise_if_mock_testing_params_disallowed rejects these before a request is routed
const GATED_MOCK_PARAMS: [&str; 2] = ["mock_delay", "mock_timeout"];

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum EmbeddingInput {
    Text(String),
    Texts(Vec<String>),
    Tokens(Vec<u32>),
    TokenLists(Vec<Vec<u32>>),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct EmbeddingRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub input: EmbeddingInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<EncodingFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(flatten)]
    pub passthrough: IndexMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EncodingFormat {
    Float,
    Base64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct EmbeddingResponse {
    pub object: String,
    pub data: Vec<EmbeddingRecord>,
    pub model: String,
    pub usage: EmbeddingUsage,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct EmbeddingRecord {
    pub object: String,
    pub embedding: EmbeddingVector,
    pub index: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum EmbeddingVector {
    Base64(String),
    Floats(Vec<serde_json::Number>),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct EmbeddingUsage {
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub completion_tokens_details: Option<CompletionTokensDetails>,
    #[serde(default)]
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CompletionTokensDetails {
    pub reasoning_tokens: Option<u32>,
    pub audio_tokens: Option<u32>,
    pub accepted_prediction_tokens: Option<u32>,
    pub rejected_prediction_tokens: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PromptTokensDetails {
    pub cached_tokens: Option<u32>,
    pub audio_tokens: Option<u32>,
}

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("the request body is not a valid embeddings request: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("betterlite does not support the {name} request parameter yet")]
    Unsupported { name: String },
    #[error("the request could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider answer is not an embeddings response: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the embeddings endpoint could not be built: {source}")]
    Endpoint {
        #[source]
        source: DeploymentError,
    },
    #[error("the provider could not be reached: {source}")]
    Http {
        #[source]
        source: HttpError,
    },
    #[error("the provider answered with status {}", .response.status)]
    Provider { response: Box<UpstreamResponse> },
    #[error("no model_list entry is named {model}")]
    UnknownModel { model: String },
    #[error("betterlite does not serve {model} embeddings yet")]
    UnsupportedProvider { model: String },
    #[error("no API key is set for this deployment, in litellm_settings or in OPENAI_API_KEY")]
    MissingApiKey,
    #[error("the mock embedding answer failed: {source}")]
    Mock {
        #[from]
        source: MockError,
    },
}

pub struct EmbeddingOutcome {
    pub response: EmbeddingResponse,
    pub upstream: UpstreamResponse,
    pub upstream_time: Duration,
}

impl EmbeddingRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, EmbeddingError> {
        let request: Self =
            serde_json::from_slice(bytes).map_err(|source| EmbeddingError::Body { source })?;
        if let Some(name) = request
            .passthrough
            .keys()
            .find(|name| GATED_MOCK_PARAMS.contains(&name.as_str()))
        {
            return Err(EmbeddingError::Unsupported { name: name.clone() });
        }
        Ok(request)
    }
}

impl EmbeddingResponse {
    fn for_client(mut self, requested_model: Option<&str>) -> Self {
        if let Some(model_name) = requested_model {
            model_name.clone_into(&mut self.model);
        }
        self
    }
}

pub async fn send(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &EmbeddingRequest,
) -> Result<EmbeddingOutcome, EmbeddingError> {
    let (client_response, client_tool_calls) =
        client_mock(&request.passthrough, executor.mock.permission)
            .map_err(|source| EmbeddingError::Body { source })?;
    let mock = resolve_mock(
        executor.mock.responses.get(&deployment.model_id),
        client_response.as_ref(),
        client_tool_calls.as_deref(),
        None,
    );
    // LiteLLM mocks an embedding only when mock_response itself is set, unlike the chat path
    if let Some(response) = mock.and_then(|plan| plan.response) {
        return mocked_embedding(executor, deployment, request, response);
    }
    let (model, api_key) = servable(deployment)?;
    let post = post(deployment, request, model, api_key)?;
    let started = Instant::now();
    let upstream = executor
        .client
        .post_json(post)
        .await
        .map_err(|source| EmbeddingError::Http { source })?;
    let upstream_time = started.elapsed();
    if upstream.status >= 400 {
        return Err(EmbeddingError::Provider {
            response: Box::new(upstream),
        });
    }
    let response = serde_json::from_slice::<EmbeddingResponse>(&upstream.body)
        .map_err(|source| EmbeddingError::Decode { source })?
        .for_client(request.model.as_deref());
    Ok(EmbeddingOutcome {
        response,
        upstream,
        upstream_time,
    })
}

fn mocked_embedding(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &EmbeddingRequest,
    response: &MockResponse,
) -> Result<EmbeddingOutcome, EmbeddingError> {
    let answer = (executor.mock_embedding)(Some(response), mock_model(&deployment.litellm_model))?;
    Ok(EmbeddingOutcome {
        response: answer.for_client(request.model.as_deref()),
        upstream: UpstreamResponse {
            status: 200,
            headers: HeaderMap::new(),
            body: Vec::new(),
        },
        upstream_time: Duration::ZERO,
    })
}

fn post<'key>(
    deployment: &'key ChatDeployment,
    request: &EmbeddingRequest,
    model: &str,
    api_key: &'key SecretString,
) -> Result<JsonPost<'key>, EmbeddingError> {
    let mut body =
        serde_json::to_value(request).map_err(|source| EmbeddingError::Body { source })?;
    let Value::Object(ref mut object) = body else {
        return Err(EmbeddingError::Encode {
            source: serde_json::Error::io(io::Error::other(
                "embedding request did not serialize as an object",
            )),
        });
    };
    object.insert(String::from("model"), Value::String(model.to_owned()));
    // the mock control fields are read before the call and never reach the provider
    object.remove("mock_response");
    object.remove("mock_tool_calls");
    let url = deployment
        .endpoint_for("embeddings")
        .map_err(|source| EmbeddingError::Endpoint { source })?;
    let body_bytes =
        serde_json::to_vec(&body).map_err(|source| EmbeddingError::Encode { source })?;
    Ok(JsonPost {
        url,
        bearer: api_key,
        headers: Vec::new(),
        body: body_bytes,
        timeout: deployment.timeout,
        body_limit: 0x0200_0000,
    })
}

fn servable(deployment: &ChatDeployment) -> Result<(&str, &SecretString), EmbeddingError> {
    match &deployment.provider {
        Provider::OpenAi { model } => deployment
            .api_key
            .as_ref()
            .map(|key| (model.as_str(), key))
            .ok_or(EmbeddingError::MissingApiKey),
        Provider::Unsupported { model } => Err(EmbeddingError::UnsupportedProvider {
            model: model.clone(),
        }),
    }
}

mod tests;
