use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use betterlite_http::error::HttpError;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::embedding::{EmbeddingError, EmbeddingRequest, EmbeddingResponse};
use betterlite_provider::operation::{
    EmbeddingOperation, OperationError, OperationRequest, OperationResult,
};
use betterlite_provider::prepared::deployment::Deployment;
use betterlite_provider::provider::Provider;

use super::CallError;

pub(crate) fn embedding_error(error: OperationError) -> CallError<EmbeddingError> {
    let route = match error {
        OperationError::Provider { response } => EmbeddingError::Provider { response },
        OperationError::MissingApiKey => EmbeddingError::MissingApiKey,
        OperationError::Encode { source } => EmbeddingError::Encode { source },
        OperationError::Decode { source, .. } => EmbeddingError::Decode { source },
        OperationError::UnavailableAdapter { provider }
        | OperationError::UnsupportedOperation { provider, .. } => {
            EmbeddingError::UnsupportedProvider { model: provider }
        }
        OperationError::Timeout => EmbeddingError::Http {
            source: HttpError::Timeout,
        },
        OperationError::TooLarge { limit } => EmbeddingError::Http {
            source: HttpError::TooLarge { limit },
        },
        kept @ (OperationError::Http { .. }
        | OperationError::InvalidHeader { .. }
        | OperationError::TransformRetry(_)) => {
            return CallError::Operation { source: kept };
        }
    };
    CallError::Route(route)
}

pub(crate) async fn provider_embedding(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &EmbeddingRequest,
) -> Result<(EmbeddingResponse, HeaderMap, Duration), CallError<EmbeddingError>> {
    let provider = Provider::from_deployment(deployment).map_err(embedding_error)?;
    let inputs = Deployment::from_chat(deployment);
    let operation = OperationRequest::Embedding(Box::new(EmbeddingOperation {
        model: inputs.model.clone(),
        request: request.clone(),
    }));
    let prepared = provider
        .prepare(&operation, &inputs, &executor.catalog)
        .map_err(embedding_error)?;
    let started = Instant::now();
    let client = executor
        .client
        .deployments
        .get(&deployment.model_id)
        .unwrap_or(&executor.client);
    let upstream = provider
        .execute(client, &prepared, &inputs)
        .await
        .map_err(embedding_error)?;
    let upstream_time = started.elapsed();
    let upstream_headers = upstream.headers().clone();
    let result = provider
        .decode(&prepared, upstream)
        .map_err(embedding_error)?;
    let OperationResult::Embedding(response) = result else {
        return Err(CallError::Mismatched);
    };
    Ok((*response, upstream_headers, upstream_time))
}
