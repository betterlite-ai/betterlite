use betterlite_config::sections::mock_response::selection::resolve_mock;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::controls::control_options::{
    ControlOptions, InvalidControlOption, parse_control_options,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::chat::{ChatRequest, RequestError, client_mock};
use betterlite_provider::operation::{
    ChatCompletion, ChatOperation, OperationError, OperationRequest, OperationResult,
};
use betterlite_provider::prepared::deployment::Deployment;
use betterlite_provider::provider::Provider;
use http::HeaderMap;
use std::time::{Duration, Instant};
use thiserror::Error;

pub(crate) fn control_options(
    request: &ChatRequest,
) -> Result<ControlOptions, InvalidControlOption> {
    let Some(given) = request.passthrough.get("stream_chunk_size") else {
        return ControlOptions::new(None);
    };
    let value: JsonValue = serde_json::from_value(given.clone())
        .map_err(|_error| InvalidControlOption::StreamChunkSize)?;
    parse_control_options(Some(&value))
}

pub(crate) async fn answer_chat(
    executor: &ChatExecutor,
    request: &ChatRequest,
    deployment: &ChatDeployment,
    control: ControlOptions,
) -> Result<(ChatCompletion, HeaderMap, Duration), CallError<ChatError>> {
    if !chat_mocks_resolve(executor, deployment, request)? {
        return provider_chat(executor, request, deployment, control).await;
    }
    let outcome = executor.send(deployment, request).await?;
    Ok((
        ChatCompletion::OpenAi(Box::new(outcome.response)),
        outcome.upstream.headers,
        outcome.upstream_time,
    ))
}

fn chat_mocks_resolve(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &ChatRequest,
) -> Result<bool, ChatError> {
    let (client_response, client_tool_calls) =
        client_mock(&request.passthrough, executor.mock.permission)
            .map_err(|source| ChatError::from(RequestError::Body { source }))?;
    let resolved = resolve_mock(
        executor.mock.responses.get(&deployment.model_id),
        client_response.as_ref(),
        client_tool_calls.as_deref(),
        request.choice_count,
    );
    Ok(resolved.is_some())
}

async fn provider_chat(
    executor: &ChatExecutor,
    request: &ChatRequest,
    deployment: &ChatDeployment,
    control: ControlOptions,
) -> Result<(ChatCompletion, HeaderMap, Duration), CallError<ChatError>> {
    let provider = Provider::from_deployment(deployment).map_err(chat_error)?;
    let inputs = Deployment::from_chat(deployment);
    let operation = OperationRequest::Chat(Box::new(ChatOperation {
        model: inputs.model.clone(),
        request: request.clone(),
        control,
    }));
    let prepared = provider
        .prepare(&operation, &inputs, &executor.catalog)
        .map_err(chat_error)?;
    let started = Instant::now();
    let client = executor
        .client
        .deployments
        .get(&deployment.model_id)
        .unwrap_or(&executor.client);
    let upstream = provider
        .execute(client, &prepared, &inputs)
        .await
        .map_err(chat_error)?;
    let upstream_time = started.elapsed();
    let upstream_headers = upstream.headers().clone();
    let result = provider.decode(&prepared, upstream).map_err(chat_error)?;
    let OperationResult::Chat(completion) = result else {
        return Err(CallError::Mismatched);
    };
    Ok((completion, upstream_headers, upstream_time))
}

// The route's own error where it names the failure; otherwise the provider's error, or the wrong operation, kept as it is
#[derive(Debug, Error)]
pub(crate) enum CallError<Route> {
    #[error(transparent)]
    Route(#[from] Route),
    #[error("the provider call could not run: {source}")]
    Operation {
        #[source]
        source: OperationError,
    },
    #[error("the provider answered a different operation than the one sent")]
    Mismatched,
}

pub(crate) fn chat_error(error: OperationError) -> CallError<ChatError> {
    let route = match error {
        OperationError::Provider { response } => ChatError::Provider { response },
        OperationError::MissingApiKey => ChatError::MissingApiKey,
        OperationError::Timeout => ChatError::Timeout,
        OperationError::Http { source } => ChatError::Http { source },
        OperationError::Encode { source } => ChatError::Encode { source },
        OperationError::Decode { source, .. } => ChatError::Decode { source },
        OperationError::TooLarge { .. } => ChatError::TooLarge,
        OperationError::UnavailableAdapter { provider }
        | OperationError::UnsupportedOperation { provider, .. } => {
            ChatError::UnsupportedProvider { model: provider }
        }
        OperationError::InvalidHeader { name, source } => ChatError::Http {
            source: TransportError::InvalidHeader { name, source },
        },
        OperationError::TransformRetry(source) => {
            return CallError::Operation {
                source: OperationError::TransformRetry(source),
            };
        }
    };
    CallError::Route(route)
}

pub(crate) mod embedding;
mod tests;
