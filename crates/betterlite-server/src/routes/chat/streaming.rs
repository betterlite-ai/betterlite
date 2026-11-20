use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Response};
use betterlite_config::sections::mock_response::selection::resolve_mock;
use betterlite_config::types::Config;
use betterlite_core::operation::chat::{
    ChatCall, ChatCallInput, ChatCompletionAnswer, ChatFailure, ChatResult, ChatStreamAnswer,
    TextCallMode,
};
use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::chat::{ChatRequest, RequestError, client_mock};
use betterlite_openai::completion_chat_bridge::{
    ChatEndpoint, answer, chat_endpoint, text_endpoint_model, text_request,
};
use betterlite_openai::text_completion_execution::failure::CompletionFailure;
use betterlite_openai::text_completion_execution::log::TracingCompletionLog;
use betterlite_openai::text_completion_execution::{
    CompletionAnswer, CompletionCallAnswer, CompletionExecutor, CompletionSend,
    CompletionStreamAnswer,
};
use betterlite_provider::operation::OperationResult;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::runtime::runtime_counter;
use futures_util::StreamExt as _;
use futures_util::stream;
use tokio::sync::OnceCell;

use super::CallContext;
use super::headers::stream_headers;
use crate::routes::completions::streaming::pump::{Pump, next_frame};
use crate::routes::completions::streaming::{StreamUsage, provider_stream};
use crate::server::chat_setup::provider_call::{CallError, answer_chat, control_options};

use self::mock::mock_stream;
use self::relay::{TextSource, relay_bytes};

// the existing chat implementation, reached through the dispatch pipeline's port
pub(crate) struct ChatCalls {
    pub(crate) executor: Arc<ChatExecutor>,
    pub(crate) counter: Counter,
}

impl ChatCalls {
    pub(crate) fn new(config: &Config, executor: Arc<ChatExecutor>) -> Result<Self, CountError> {
        let counter = runtime_counter(
            config,
            &Arc::new(executor.client.clone()),
            &BundledCatalog::default().get(),
        )?;
        Ok(Self { executor, counter })
    }
}

fn chat_failure(error: CallError<ChatError>) -> ChatFailure {
    match error {
        CallError::Route(route) => ChatFailure::Route(route),
        CallError::Operation { source } => ChatFailure::Internal {
            message: source.to_string(),
        },
        CallError::Mismatched => ChatFailure::Mismatched,
    }
}

#[async_trait]
impl ChatCall for ChatCalls {
    fn prepare(&self, request: &ChatRequest) -> Result<ControlOptions, ChatFailure> {
        control_options(request).map_err(ChatFailure::from)
    }

    async fn complete(
        &self,
        call: ChatCallInput<'_>,
        control: ControlOptions,
    ) -> Result<ChatCompletionAnswer, ChatFailure> {
        let (result, upstream_headers, upstream_time) =
            answer_chat(&self.executor, call.request, &call.deployment, control)
                .await
                .map_err(chat_failure)?;
        let (completion, upstream_headers) = if let OperationResult::OutputLimit { sent } = &result
        {
            recovery::completion(self, sent, upstream_headers, call.request.model.as_deref())
                .await?
        } else {
            let OperationResult::Chat(completion) = result else {
                return Err(ChatFailure::Mismatched);
            };
            (completion, upstream_headers)
        };
        let body = serde_json::to_vec(&completion)
            .map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
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
        let executor = &self.executor;
        let (client_response, client_tool_calls) =
            client_mock(&call.request.passthrough, executor.mock.permission).map_err(|source| {
                ChatFailure::Route(ChatError::from(RequestError::Body { source }))
            })?;
        let resolved = resolve_mock(
            executor.mock.responses.get(&call.deployment.model_id),
            client_response.as_ref(),
            client_tool_calls.as_deref(),
            call.request.choice_count,
        );
        if let Some(plan) = resolved {
            return mock_stream(executor, &call.deployment, call.request, &plan);
        }
        let started = Instant::now();
        let (result, upstream_headers) = provider_stream(
            executor,
            &call.deployment,
            call.request,
            control,
            StreamUsage::Chat,
        )
        .await
        .map_err(chat_failure)?;
        let upstream_time = started.elapsed();
        if let OperationResult::OutputLimit { sent } = &result {
            return recovery::stream(self, sent, upstream_headers, upstream_time).await;
        }
        let OperationResult::Stream(stream) = result else {
            return Err(ChatFailure::Mismatched);
        };
        Ok(ChatStreamAnswer {
            body: stream.body_stream(),
            upstream_headers,
            upstream_time,
        })
    }

    // main.py sends a text-completion deployment's chat request through the text endpoint
    fn endpoint(&self, deployment: &ChatDeployment) -> ChatEndpoint {
        chat_endpoint(deployment)
    }

    // The text endpoint's answer in the shape the route asked for; no provider headers reach the client.
    async fn text(
        &self,
        call: ChatCallInput<'_>,
        mode: TextCallMode,
    ) -> Result<ChatResult, ChatFailure> {
        let streamed = matches!(mode, TextCallMode::Streamed);
        let answered = bridged_send(self, &call, streamed).await?;
        answered_result(&self.executor, mode, answered, call.request)
    }
}

fn answered_result(
    executor: &ChatExecutor,
    mode: TextCallMode,
    answered: CompletionCallAnswer,
    request: &ChatRequest,
) -> Result<ChatResult, ChatFailure> {
    match (mode, answered) {
        (TextCallMode::Buffered { convert }, CompletionCallAnswer::Buffered(answer)) => {
            let created = i64::try_from((executor.clock)()).unwrap_or(i64::MAX);
            buffered_result(&answer, convert, request.model.as_deref(), created)
        }
        (TextCallMode::Streamed, CompletionCallAnswer::Streamed(answer)) => {
            let include_usage = request
                .stream_options
                .as_ref()
                .is_some_and(|chosen| chosen.include_usage);
            let model = request.model.clone().unwrap_or_default();
            Ok(streamed_result(answer, model, include_usage))
        }
        _ => Err(ChatFailure::Mismatched),
    }
}

fn buffered_result(
    answer: &CompletionAnswer,
    convert: bool,
    asked: Option<&str>,
    created: i64,
) -> Result<ChatResult, ChatFailure> {
    let body = if convert {
        answer::chat_completion_body(&answer.response, created, asked)
    } else {
        answer::text_completion_body(&answer.response, asked)
    }
    .map_err(completion_failure)?;
    Ok(ChatResult::Completion(ChatCompletionAnswer {
        body,
        upstream_headers: HeaderMap::new(),
        upstream_time: answer.upstream_time,
    }))
}

fn streamed_result(
    answer: CompletionStreamAnswer,
    model: String,
    include_usage: bool,
) -> ChatResult {
    let source = TextSource {
        stream: answer.stream,
        model,
        include_usage,
    };
    ChatResult::Stream(ChatStreamAnswer {
        body: stream::unfold(Pump::new(source), next_frame)
            .map(|item| item.map(|bytes| bytes.to_vec()))
            .boxed(),
        upstream_headers: HeaderMap::new(),
        upstream_time: answer.upstream_time,
    })
}

async fn bridged_send(
    calls: &ChatCalls,
    call: &ChatCallInput<'_>,
    streamed: bool,
) -> Result<CompletionCallAnswer, ChatFailure> {
    let model = text_endpoint_model(&call.deployment).ok_or(ChatFailure::Mismatched)?;
    let transport =
        text_request(&call.deployment, call.request, &model).map_err(completion_failure)?;
    let mut logger = TracingCompletionLog;
    let executor = CompletionExecutor {
        client: calls.executor.client.clone(),
        bundled_catalog: Arc::clone(&calls.executor.bundled_catalog),
        catalog: OnceCell::new(),
    };
    executor
        .send(CompletionSend {
            deployment: &call.deployment,
            request: transport,
            streamed,
            redactor: &calls.executor.redactor,
            logger: &mut logger,
        })
        .await
        .map_err(completion_failure)
}

// the chat route answers with the provider's own error envelope, as LiteLLM's exception path does
fn completion_failure(failure: CompletionFailure) -> ChatFailure {
    let body = ErrorResponse {
        error: ErrorObject {
            message: failure.message,
            error_type: failure.error_type.map_or(WireField::Null, WireField::Text),
            param: failure.param.map_or(WireField::Null, WireField::Text),
            code: WireField::Null,
        },
    };
    match serde_json::to_vec(&body) {
        Ok(encoded) => ChatFailure::Route(ChatError::Provider {
            response: Box::new(UpstreamResponse {
                status: failure.status,
                headers: HeaderMap::new(),
                body: encoded,
            }),
        }),
        Err(source) => ChatFailure::Route(ChatError::Encode { source }),
    }
}

pub(super) fn render_stream(
    answer: ChatStreamAnswer,
    deployment: &ChatDeployment,
    request: &ChatRequest,
    call: &CallContext<'_>,
) -> Response {
    let headers = stream_headers(
        &answer.upstream_headers,
        deployment,
        call.id,
        answer.upstream_time,
        call.started.elapsed(),
    );
    let stream_body = Body::from_stream(relay_bytes(answer.body, request));
    (headers, stream_body).into_response()
}

mod mock;
mod recovery;
mod relay;
mod tests;
