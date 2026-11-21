use std::collections::BTreeMap;

use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::perplexity_embedding;
use betterlite_openai::perplexity_embedding::error::PerplexityEmbeddingError;
use betterlite_openai::perplexity_embedding::reply::transform_embedding_response;
use betterlite_prompts::arguments::json::Object;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method};
use serde::de::Error as _;
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
        let decoded = serde_json::to_vec(&reply)
            .and_then(|bytes| serde_json::from_slice::<EmbeddingResponse>(&bytes))
            .map_err(|source| OperationError::Decode {
                operation: "embedding",
                source,
            })?;
        Ok(OperationResult::Embedding(Box::new(decoded)))
    }
}

fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from(Perplexity::NAME),
        operation: String::from("provider endpoint"),
    }
}

// only dimensions and encoding_format survive map_openai_params, so the rest of the request never reaches the body
fn prepared_embedding(
    operation: &EmbeddingOperation,
    deployment: &Deployment,
) -> Result<PreparedRequest, OperationError> {
    let requested: Object = serde_json::to_vec(&operation.request)
        .and_then(|bytes| serde_json::from_slice(&bytes))
        .map_err(|source| OperationError::Encode { source })?;
    let mut optional_params = Object::new();
    perplexity_embedding::map_openai_params(&requested, &mut optional_params);
    let call = EmbeddingCall {
        model: deployment.model.clone(),
        input: operation.request.input.clone(),
        optional_params,
        litellm_params: Object::new(),
        api_key: deployment.api_key.clone(),
        api_base: Some(deployment.api_base.clone()),
        headers: None,
        timeout: deployment.timeout,
        aembedding: false,
    };
    let Ok(resolved) = embedding(&call) else {
        return Err(unsupported());
    };
    let body = perplexity_embedding::transform_embedding_request(
        &resolved.model,
        &resolved.input,
        &resolved.optional_params,
    )
    .map_err(|error| OperationError::Encode {
        source: serde_json::Error::custom(error),
    })?;
    let url = perplexity_embedding::get_complete_url(resolved.api_base.as_deref());
    // the proxy hands no process environment to an adapter, so only the deployment's own key is read
    let wire_headers = perplexity_embedding::validate_environment(
        resolved.api_key.as_ref(),
        &BTreeMap::<String, String>::new(),
    );
    let mut headers = HeaderMap::new();
    if let Some(bearer) = wire_headers.get("Authorization") {
        headers.insert(AUTHORIZATION, authorization_value(bearer)?);
    }
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let http = PreparedHttp {
        method: Method::POST,
        url: Url::parse(&url).map_err(|source| OperationError::Encode {
            source: serde_json::Error::custom(source),
        })?,
        headers,
        body: serde_json::to_vec(&body).map_err(|source| OperationError::Encode { source })?,
        deadline: resolved.timeout,
        body_limit: None,
    };
    Ok(PreparedRequest::Embedding(Box::new(PreparedChat {
        http,
        retry: TransformRetry::DISABLED,
        client_model: Some(resolved.model),
    })))
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
            source: serde_json::Error::custom(source),
        },
    }
}
