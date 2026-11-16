use axum::http::HeaderMap;
use betterlite_config::sections::mock_response::selection::resolve_mock;
use betterlite_openai::chat::client_mock;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::embedding::{self, EmbeddingError, EmbeddingRequest, EmbeddingResponse};
use std::time::Duration;

use crate::server::chat_setup::provider_call::CallError;
use crate::server::chat_setup::provider_call::embedding::provider_embedding;

pub(super) async fn embed(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &EmbeddingRequest,
) -> Result<(EmbeddingResponse, HeaderMap, Duration), CallError<EmbeddingError>> {
    if embedding_mocks_resolve(executor, deployment, request)? {
        let outcome = embedding::send(executor, deployment, request).await?;
        return Ok((
            outcome.response,
            outcome.upstream.headers,
            outcome.upstream_time,
        ));
    }
    provider_embedding(executor, deployment, request).await
}

pub(super) fn embedding_mocks_resolve(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &EmbeddingRequest,
) -> Result<bool, EmbeddingError> {
    let (client_response, client_tool_calls) =
        client_mock(&request.passthrough, executor.mock.permission)
            .map_err(|source| EmbeddingError::Body { source })?;
    let resolved = resolve_mock(
        executor.mock.responses.get(&deployment.model_id),
        client_response.as_ref(),
        client_tool_calls.as_deref(),
        None,
    );
    Ok(resolved.and_then(|plan| plan.response).is_some())
}
