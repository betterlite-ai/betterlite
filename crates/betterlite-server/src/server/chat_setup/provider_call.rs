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
use serde_json::Error as JsonError;
use std::io::Error as IoError;
use std::time::{Duration, Instant};

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

pub(crate) fn chat_mocks_resolve(
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

pub(crate) async fn provider_chat(
    executor: &ChatExecutor,
    request: &ChatRequest,
    deployment: &ChatDeployment,
    control: ControlOptions,
) -> Result<(ChatCompletion, HeaderMap, Duration), ChatError> {
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
        return Err(mismatched());
    };
    Ok((completion, upstream_headers, upstream_time))
}

pub(crate) fn mismatched() -> ChatError {
    ChatError::Decode {
        source: JsonError::io(IoError::other(
            "the provider answered a different operation",
        )),
    }
}

pub(crate) fn chat_error(error: OperationError) -> ChatError {
    match error {
        OperationError::Provider { response } => ChatError::Provider { response },
        OperationError::MissingApiKey => ChatError::MissingApiKey,
        OperationError::Timeout => ChatError::Timeout,
        OperationError::Http { source } => ChatError::Http { source },
        OperationError::Encode { source } => ChatError::Encode { source },
        OperationError::Decode { source, .. } => ChatError::Decode { source },
        OperationError::TooLarge => ChatError::TooLarge,
        OperationError::UnavailableAdapter { provider }
        | OperationError::UnsupportedOperation { provider, .. } => {
            ChatError::UnsupportedProvider { model: provider }
        }
        OperationError::InvalidHeader { name, source } => ChatError::Http {
            source: TransportError::InvalidHeader { name, source },
        },
        OperationError::Endpoint { value, source } => ChatError::Encode {
            source: JsonError::io(IoError::other(format!("{value}: {source}"))),
        },
        OperationError::Deployment(source) => ChatError::Encode {
            source: JsonError::io(IoError::other(source.to_string())),
        },
        OperationError::TransformRetry(source) => ChatError::Encode {
            source: JsonError::io(IoError::other(source.to_string())),
        },
    }
}

mod tests;
