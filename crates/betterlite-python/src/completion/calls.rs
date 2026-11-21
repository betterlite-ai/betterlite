use async_trait::async_trait;
use betterlite_core::operation::chat::{
    ChatCall, ChatCallInput, ChatCompletionAnswer, ChatFailure, ChatStreamAnswer,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::controls::control_options::{
    ControlOptions, InvalidControlOption, parse_control_options,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::params::ParameterError;
use betterlite_openai::chat::execution::ChatError;
use betterlite_openai::chat::{ChatRequest, RequestError};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_provider::operation::{
    ChatOperation, OperationError, OperationRequest, OperationResult,
};
use betterlite_provider::prepared::deployment::Deployment;
use betterlite_provider::provider::Provider;
use http::HeaderMap;
use std::time::{Duration, Instant};
// The provider framework the server's chat route uses, reached from a Python session instead of a router.
pub(crate) struct ProviderCalls {
    pub(crate) client: ProxiedClient,
    pub(crate) catalog: CatalogCapabilities,
}
impl ProviderCalls {
    // main.py hands stream_chunk_size to the streaming writer; the route's own parser validates it
    fn control(&self, request: &ChatRequest) -> Result<ControlOptions, ChatFailure> {
        let Some(given) = request.passthrough.get("stream_chunk_size") else {
            return Ok(ControlOptions::default());
        };
        let value: JsonValue = serde_json::from_value(given.clone()).map_err(|_error| {
            ChatFailure::InvalidControl {
                source: InvalidControlOption::StreamChunkSize,
            }
        })?;
        parse_control_options(Some(&value)).map_err(|source| ChatFailure::InvalidControl { source })
    }
    async fn send(
        &self,
        call: &ChatCallInput<'_>,
        control: ControlOptions,
        streaming: bool,
    ) -> Result<(OperationResult, HeaderMap, Duration), ChatFailure> {
        let deployment = &call.deployment;
        let provider = Provider::from_deployment(deployment).map_err(chat_failure)?;
        let inputs = Deployment::from_chat(deployment);
        let operation = ChatOperation {
            model: inputs.model.clone(),
            request: call.request.clone(),
            control,
        };
        let request = if streaming {
            OperationRequest::Stream(Box::new(operation))
        } else {
            OperationRequest::Chat(Box::new(operation))
        };
        let prepared = provider
            .prepare(&request, &inputs, &self.catalog)
            .map_err(chat_failure)?;
        let started = Instant::now();
        let upstream = provider
            .execute(&self.client, &prepared, &inputs)
            .await
            .map_err(chat_failure)?;
        let upstream_time = started.elapsed();
        let upstream_headers = upstream.headers().clone();
        let result = provider.decode(&prepared, upstream).map_err(chat_failure)?;
        Ok((result, upstream_headers, upstream_time))
    }
}
#[async_trait]
impl ChatCall for ProviderCalls {
    fn prepare(&self, request: &ChatRequest) -> Result<ControlOptions, ChatFailure> {
        self.control(request)
    }
    async fn complete(
        &self,
        call: ChatCallInput<'_>,
        control: ControlOptions,
    ) -> Result<ChatCompletionAnswer, ChatFailure> {
        let (result, upstream_headers, upstream_time) = self.send(&call, control, false).await?;
        let OperationResult::Chat(completion) = result else {
            return Err(ChatFailure::Mismatched);
        };
        let body = serde_json::to_vec(&completion).map_err(|source| ChatFailure::Internal {
            message: source.to_string(),
        })?;
        Ok(ChatCompletionAnswer {
            body,
            upstream_headers,
            upstream_time,
        })
    }
    async fn stream(
        &self,
        call: ChatCallInput<'_>,
        control: ControlOptions,
    ) -> Result<ChatStreamAnswer, ChatFailure> {
        let (result, upstream_headers, upstream_time) = self.send(&call, control, true).await?;
        let OperationResult::Stream(stream) = result else {
            return Err(ChatFailure::Mismatched);
        };
        Ok(ChatStreamAnswer {
            body: stream.body_stream(),
            upstream_headers,
            upstream_time,
        })
    }
}
// the provider framework's error, carried into the chat port the way the server's chat route does it
fn chat_failure(error: OperationError) -> ChatFailure {
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
        OperationError::Parameters {
            source: ParameterError::Unsupported { names, .. },
        } => ChatError::Request {
            source: RequestError::Unsupported {
                name: names.join(", "),
            },
        },
        OperationError::Parameters {
            source: ParameterError::Encode { source },
        } => ChatError::Request {
            source: RequestError::Body { source },
        },
        kept @ (OperationError::TransformRetry(_)
        | OperationError::Parameters { .. }
        | OperationError::RejectedFields { .. }
        | OperationError::Anthropic { .. }) => {
            return ChatFailure::Internal {
                message: kept.to_string(),
            };
        }
    };
    ChatFailure::Route(route)
}
