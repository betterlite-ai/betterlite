use std::collections::BTreeMap;
use std::io::Error as IoError;

use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::controls::call_params::normalize_drop_params;
use betterlite_models::params::ParameterError;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::perplexity_embedding;
use betterlite_openai::perplexity_embedding::error::PerplexityEmbeddingError;
use betterlite_openai::perplexity_embedding::reply::transform_embedding_response;
use betterlite_prompts::arguments::json::{Json, Object};
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method};
use secrecy::SecretString;
use thiserror::Error;
use url::Url;

use super::embedding;
use crate::hosted_platform_dispatch::call::EmbeddingCall;
use crate::operation::{
    EmbeddingOperation, OperationError, OperationRequest, OperationResult, RequestKind, success,
};
use crate::prepared::body::authorization_value;
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedKind, PreparedRequest, Upstream};
use crate::provider::Adapter;
use crate::transform_retry::TransformRetry;

// main.py sends a perplexity/ embedding through the shared HTTP handler with PerplexityEmbeddingConfig
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Perplexity;

// LiteLLM's DEFAULT_EMBEDDING_PARAM_VALUES: every other request field is collected into extra_body
const DEFAULT_EMBEDDING_PARAMS: [&str; 6] = [
    "dimensions",
    "encoding_format",
    "user",
    "model",
    "custom_llm_provider",
    "input",
];

// serde_json carries the transform and reply failures, so each leaf error travels as the typed cause it wraps
#[derive(Debug, Error)]
enum LeafError {
    #[error(transparent)]
    Transform(PerplexityEmbeddingError),
    #[error(transparent)]
    Url(url::ParseError),
    #[error(transparent)]
    Base64(base64::DecodeError),
}

impl Adapter for Perplexity {
    const NAME: &'static str = "perplexity";

    fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        _catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        let RequestKind::Embedding(operation) = request.kind() else {
            return Err(unsupported());
        };
        prepared_embedding(operation, deployment)
    }

    async fn execute(
        &self,
        client: &ProxiedClient,
        prepared: &PreparedRequest,
        _deployment: &Deployment,
    ) -> Result<Upstream, OperationError> {
        prepared.send(client).await
    }

    fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        let (PreparedKind::Embedding(sent), Upstream::Buffered(response)) =
            (prepared.kind(), upstream)
        else {
            return Err(unsupported());
        };
        let checked = success(response)?;
        let model = sent.client_model.as_deref().unwrap_or_default();
        let reply = match transform_embedding_response(model, checked.status, &checked.body) {
            Ok(reply) => reply,
            Err(error) => return Err(reply_error(error, checked)),
        };
        let mut decoded = serde_json::to_vec(&reply)
            .and_then(|bytes| serde_json::from_slice::<EmbeddingResponse>(&bytes))
            .map_err(|source| OperationError::Decode {
                operation: "embedding",
                source,
            })?;
        if let Some(reply_model) = &sent.client_model {
            reply_model.clone_into(&mut decoded.model);
        }
        Ok(OperationResult::Embedding(Box::new(decoded)))
    }
}

fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from(Perplexity::NAME),
        operation: String::from("provider endpoint"),
    }
}

// LiteLLM's get_optional_params_embeddings: a set user is refused unless drop_params drops it, and every other request field plus a client extra_body is merged over the transformed body
fn prepared_embedding(
    operation: &EmbeddingOperation,
    deployment: &Deployment,
) -> Result<PreparedRequest, OperationError> {
    let requested: Object = serde_json::to_vec(&operation.request)
        .and_then(|bytes| serde_json::from_slice(&bytes))
        .map_err(|source| OperationError::Encode { source })?;
    let call = embedding_call(operation, &requested, deployment);
    let resolved = embedding(&call);
    check_embedding_params(operation, deployment, &resolved.model)?;
    let mut body = perplexity_embedding::transform_embedding_request(
        &resolved.model,
        &resolved.input,
        &resolved.optional_params,
    )
    .map_err(|error| OperationError::Encode {
        source: serde_json::Error::io(IoError::other(LeafError::Transform(error))),
    })?;
    body.extend(embedding_extra_body(&requested)?);
    let http = PreparedHttp {
        method: Method::POST,
        url: embedding_url(&resolved)?,
        headers: embedding_headers(resolved.api_key.as_ref())?,
        body: serde_json::to_vec(&body).map_err(|source| OperationError::Encode { source })?,
        deadline: resolved.timeout,
        body_limit: None,
    };
    let client_model = operation.request.model.clone().unwrap_or(resolved.model);
    Ok(PreparedRequest::Embedding(Box::new(PreparedChat {
        http,
        retry: TransformRetry::DISABLED,
        client_model: Some(client_model),
    })))
}

// LiteLLM's _check_valid_arg over OPENAI_EMBEDDING_PARAMS: only dimensions and encoding_format pass, and drop_params allows the rest to be dropped
fn check_embedding_params(
    operation: &EmbeddingOperation,
    deployment: &Deployment,
    model: &str,
) -> Result<(), OperationError> {
    let drop_params = deployment.global_drop_params
        || normalize_drop_params(deployment.drop_params.as_ref()).unwrap_or(false);
    let unsupported: Vec<String> = [
        ("dimensions", operation.request.dimensions.is_some()),
        (
            "encoding_format",
            operation.request.encoding_format.is_some(),
        ),
        ("user", operation.request.user.is_some()),
    ]
    .into_iter()
    .filter(|(name, set)| *set && !perplexity_embedding::SUPPORTED_OPENAI_PARAMS.contains(name))
    .map(|(name, _set)| String::from(name))
    .collect();
    if unsupported.is_empty() || drop_params {
        return Ok(());
    }
    Err(OperationError::Parameters {
        source: ParameterError::Unsupported {
            model: String::from(model),
            names: unsupported,
        },
    })
}

// LiteLLM's add_provider_specific_params_to_optional_params: the client extra_body and every field outside the defaults, merged over the body after the transform
fn embedding_extra_body(requested: &Object) -> Result<Object, OperationError> {
    let mut extra_body = Object::new();
    match requested.get("extra_body") {
        None | Some(Json::Null) => {}
        Some(Json::Object(client)) => {
            for (name, value) in client {
                // EXTRA_BODY_ROUTING_KEYS: a client extra_body never replaces the request's own model
                if name != "model" {
                    extra_body.insert(name.clone(), value.clone());
                }
            }
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(OperationError::Parameters {
                source: ParameterError::NotAnObject {
                    name: String::from("extra_body"),
                },
            });
        }
    }
    for (name, value) in requested {
        // LiteLLM drops a null extra and never collects a named embedding param into extra_body
        if name == "extra_body"
            || *value == Json::Null
            || DEFAULT_EMBEDDING_PARAMS.contains(&name.as_str())
        {
            continue;
        }
        extra_body.insert(name.clone(), value.clone());
    }
    Ok(extra_body)
}

// the embedding URL rules of get_complete_url, with a parse failure as a typed encode cause
fn embedding_url(resolved: &EmbeddingCall) -> Result<Url, OperationError> {
    Url::parse(&perplexity_embedding::get_complete_url(
        resolved.api_base.as_deref(),
    ))
    .map_err(|source| OperationError::Encode {
        source: serde_json::Error::io(IoError::other(LeafError::Url(source))),
    })
}

fn embedding_call(
    operation: &EmbeddingOperation,
    requested: &Object,
    deployment: &Deployment,
) -> EmbeddingCall {
    let mut optional_params = Object::new();
    perplexity_embedding::map_openai_params(requested, &mut optional_params);
    EmbeddingCall {
        model: deployment.model.clone(),
        input: operation.request.input.clone(),
        optional_params,
        litellm_params: Object::new(),
        api_key: deployment.api_key.clone(),
        api_base: Some(deployment.api_base.clone()),
        headers: None,
        timeout: deployment.timeout,
        aembedding: false,
    }
}

// the proxy hands no process environment to an adapter, so only the deployment's own key is read
fn embedding_headers(api_key: Option<&SecretString>) -> Result<HeaderMap, OperationError> {
    let wire =
        perplexity_embedding::validate_environment(api_key, &BTreeMap::<String, String>::new());
    let mut headers = HeaderMap::new();
    if let Some(bearer) = wire.get("Authorization") {
        headers.insert(AUTHORIZATION, authorization_value(bearer)?);
    }
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}

// a non-JSON reply keeps its status and text the way PerplexityEmbeddingError carries them
fn reply_error(error: PerplexityEmbeddingError, response: UpstreamResponse) -> OperationError {
    match error {
        PerplexityEmbeddingError::Provider { .. } => OperationError::Provider {
            response: Box::new(response),
        },
        PerplexityEmbeddingError::TooLarge { limit } => OperationError::TooLarge { limit },
        PerplexityEmbeddingError::Shape { source }
        | PerplexityEmbeddingError::Encode { source } => OperationError::Decode {
            operation: "embedding",
            source,
        },
        PerplexityEmbeddingError::Base64 { source } => OperationError::Decode {
            operation: "embedding",
            source: serde_json::Error::io(IoError::other(LeafError::Base64(source))),
        },
    }
}
