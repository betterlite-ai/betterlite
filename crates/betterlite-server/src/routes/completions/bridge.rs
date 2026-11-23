use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use betterlite_core::operation::text_completion::{
    ChatCallAnswer, ChatCallMode, TextCompletionCallInput, TextCompletionFailure,
    TextCompletionResult,
};
use betterlite_core::operation::text_stream::TextStreamAnswer;
use betterlite_http::error::HttpError;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatRequest, execution::ChatError};
use betterlite_openai::completion_chat_bridge::{answer::text_completion_response, chat_request};
use betterlite_openai::stream::SseParser;
use betterlite_openai::text_completion_execution::{
    CompletionAnswer, CompletionCallAnswer, CompletionSend, failure::CompletionFailure,
    log::TracingCompletionLog,
};
use betterlite_openai::text_completion_stream::shaped_chunk;
use betterlite_provider::anthropic_dispatch::AnthropicResult;
use betterlite_provider::anthropic_dispatch::gateway::reply::AnthropicReply;
use betterlite_provider::anthropic_dispatch::gateway::runtime::AnthropicRuntime;
use betterlite_provider::operation::{ChatCompletion, OperationResult};
use betterlite_provider::provider::Provider;
use futures_util::stream::{BoxStream, StreamExt as _};

use super::CompletionCalls;
use super::streaming::chunks::frames_answer;
use super::streaming::{self, StreamRoute, StreamUsage, provider_stream};
use crate::server::chat_setup::provider_call::{CallError, Runtimes, answer_chat};

const MISMATCHED: &str = "the provider answered a different operation";

pub(super) async fn text_answer(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
) -> Result<TextCompletionResult, TextCompletionFailure> {
    let CompletionCallAnswer::Buffered(answer) = text_send(calls, call, false).await? else {
        return Err(TextCompletionFailure::internal(MISMATCHED));
    };
    Ok(text_result(answer))
}

pub(super) fn converted(
    completion: ChatCompletion,
    upstream_headers: HeaderMap,
    upstream_time: Duration,
) -> Result<CompletionAnswer, TextCompletionFailure> {
    let ChatCompletion::OpenAi(response) = completion else {
        return Err(TextCompletionFailure::internal(MISMATCHED));
    };
    Ok(CompletionAnswer {
        response: Box::new(text_completion_response(*response)),
        upstream_headers,
        upstream_time,
    })
}

// which reader each upstream frame set goes through: the shared chat chunk reader, or the Anthropic adapter's own chunks
#[derive(Clone, Copy)]
pub(super) enum ChunkUsage {
    Chat(bool),
    Anthropic,
}

// streaming::chunks holds the frame source; the two names stay here because the completion tests build the reader from bridge
pub(super) struct ChatChunks {
    pub(super) bytes: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    pub(super) parser: SseParser,
    pub(super) model: String,
    pub(super) usage: ChunkUsage,
}

// the chat endpoint reached for a chat-catalog deployment, with the one Anthropic runtime the application built
pub(super) async fn chat_bridge(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
    mode: ChatCallMode,
) -> Result<ChatCallAnswer, TextCompletionFailure> {
    let runtime = calls.anthropic()?;
    let chat = chat_request(call.request).map_err(bridge_failure)?;
    let control = ControlOptions::default();
    if matches!(mode, ChatCallMode::Streamed) {
        let stream = streamed_answer(calls, call, &chat, control, runtime).await?;
        return Ok(ChatCallAnswer::Streamed(stream));
    }
    let executor = &calls.state.executor;
    let answered = answer_chat(
        executor,
        &chat,
        &call.deployment,
        control,
        Runtimes {
            anthropic: Some(runtime),
            azure: &calls.state.azure_runtime.composition,
        },
    )
    .await;
    let (result, headers, time) = answered.map_err(|error| chat_call_failure(error, calls))?;
    let answer = if let OperationResult::Anthropic(AnthropicResult::Reply(reply)) = result {
        anthropic_converted(reply, headers, time)?
    } else {
        streaming::buffered(calls, call.request, (result, headers, time)).await?
    };
    Ok(ChatCallAnswer::Buffered(text_result(answer)))
}

// Only the Anthropic adapter reads the runtime, so every other deployment keeps the route's stream, recovery included.
async fn streamed_answer(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
    chat: &ChatRequest,
    control: ControlOptions,
    runtime: &Arc<AnthropicRuntime>,
) -> Result<TextStreamAnswer, TextCompletionFailure> {
    let Ok(Provider::Anthropic(_)) =
        Provider::from_deployment(&call.deployment, &calls.state.azure_runtime.composition)
    else {
        return streaming::streamed(calls, call, chat, control).await;
    };
    let executor = &calls.state.executor;
    let route = StreamRoute {
        usage: StreamUsage::Text,
        anthropic: Some(runtime),
        azure: &calls.state.azure_runtime.composition,
    };
    let started = Instant::now();
    let answered = provider_stream(executor, &call.deployment, chat, control, route).await;
    let (result, headers) = answered.map_err(|error| chat_call_failure(error, calls))?;
    let OperationResult::Anthropic(AnthropicResult::Stream(frames)) = result else {
        return Err(TextCompletionFailure::internal(MISMATCHED));
    };
    let model = call.request.model.as_deref();
    let usage = ChunkUsage::Anthropic;
    let answer = frames_answer(frames, headers, model, usage, started.elapsed());
    Ok(answer)
}

// The Messages reply read back as the chat response LiteLLM's text conversion takes; its tool calls and thinking drop out.
pub(super) fn anthropic_converted(
    reply: Box<AnthropicReply>,
    upstream_headers: HeaderMap,
    upstream_time: Duration,
) -> Result<CompletionAnswer, TextCompletionFailure> {
    let mut dump = serde_json::to_value(reply)
        .map_err(|source| TextCompletionFailure::internal(&source.to_string()))?;
    // the chat response cannot read Anthropic's usage details and would drop its extras, so the reply's own dump is restored
    let usage = dump.as_object_mut().and_then(|map| map.remove("usage"));
    let chat = serde_json::from_value(dump)
        .map_err(|source| TextCompletionFailure::internal(&source.to_string()))?;
    let mut response = text_completion_response(chat);
    response.usage = serde_json::from_value(usage.unwrap_or_default())
        .map_err(|source| TextCompletionFailure::internal(&source.to_string()))?;
    Ok(CompletionAnswer {
        response: Box::new(response),
        upstream_headers,
        upstream_time,
    })
}

pub(super) async fn text_stream(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
) -> Result<TextStreamAnswer, TextCompletionFailure> {
    let CompletionCallAnswer::Streamed(answer) = text_send(calls, call, true).await? else {
        return Err(TextCompletionFailure::internal(MISMATCHED));
    };
    let named = call.request.model.as_deref().unwrap_or_default().to_owned();
    let body = answer
        .stream
        .map(move |item| {
            let chunk =
                item.map_err(|error| TextCompletionFailure::new(500, None, error.to_string()))?;
            let value = shaped_chunk(chunk, &named)
                .map_err(|source| TextCompletionFailure::new(500, None, source.to_string()))?;
            serde_json::from_value::<Json>(value)
                .map_err(|source| TextCompletionFailure::new(500, None, source.to_string()))
        })
        .boxed();
    Ok(TextStreamAnswer {
        body,
        upstream_headers: answer.upstream_headers,
        upstream_time: answer.upstream_time,
    })
}

async fn text_send(
    calls: &CompletionCalls,
    call: &TextCompletionCallInput<'_>,
    streamed: bool,
) -> Result<CompletionCallAnswer, TextCompletionFailure> {
    let execution = &calls.execution;
    let transport = execution.text_transport(&call.deployment, call.request, streamed);
    let built = transport.await.map_err(bridge_failure)?;
    let mut logger = TracingCompletionLog;
    let sent = execution.send(CompletionSend {
        deployment: &call.deployment,
        request: built,
        streamed,
        redactor: &calls.state.executor.redactor,
        logger: &mut logger,
    });
    sent.await.map_err(bridge_failure)
}

pub(super) fn text_result(answer: CompletionAnswer) -> TextCompletionResult {
    TextCompletionResult {
        response: answer.response,
        upstream_headers: answer.upstream_headers,
        upstream_time: answer.upstream_time,
    }
}

pub(super) fn bridge_failure(failure: CompletionFailure) -> TextCompletionFailure {
    TextCompletionFailure {
        status: failure.status,
        error_type: failure.error_type,
        param: failure.param,
        message: failure.message,
    }
}

pub(super) fn chat_call_failure(
    error: CallError<ChatError>,
    calls: &CompletionCalls,
) -> TextCompletionFailure {
    match error {
        CallError::Route(route) => {
            let failure = route.failure(&calls.state.executor.redactor);
            TextCompletionFailure {
                status: failure.status,
                error_type: Some(failure.error_type),
                param: failure.param,
                message: route.to_string(),
            }
        }
        CallError::Operation { source } => TextCompletionFailure::internal(&source.to_string()),
        CallError::Mismatched => TextCompletionFailure::internal(MISMATCHED),
    }
}
