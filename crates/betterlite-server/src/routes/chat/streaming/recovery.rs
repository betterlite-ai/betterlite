use std::time::Duration;

use axum::http::HeaderMap;
use betterlite_core::dispatch::chat::mock_stream::sse_frame;
use betterlite_core::operation::chat::{ChatFailure, ChatStreamAnswer};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::output_limits::build::{
    OutputLimitResponse, RecoveredBody, Recovery, build_limit_response,
};
use betterlite_openai::stream::chunk::{Chunk, Delta};
use betterlite_provider::operation::{ChatCompletion, OperationResult};
use betterlite_tokenizer::count::Counter;
use futures_util::StreamExt as _;
use futures_util::stream;

use super::ChatCalls;

// the buffered answer for a decoded result: the recovered completion, or the provider's own answer
pub(crate) async fn completed(
    calls: &ChatCalls,
    result: OperationResult,
    headers: HeaderMap,
    client_model: Option<&str>,
) -> Result<(ChatCompletion, HeaderMap), ChatFailure> {
    if let OperationResult::OutputLimit { sent } = &result {
        return recovered_completion(&calls.executor, &calls.counter, sent, headers, client_model)
            .await;
    }
    let OperationResult::Chat(completion) = result else {
        return Err(ChatFailure::Mismatched);
    };
    Ok((completion, headers))
}

// the same answer for a route that holds the executor and the token counter itself
pub(crate) async fn recovered_completion(
    executor: &ChatExecutor,
    tokens: &Counter,
    sent: &JsonValue,
    headers: HeaderMap,
    client_model: Option<&str>,
) -> Result<(ChatCompletion, HeaderMap), ChatFailure> {
    let answer = recover(executor, tokens, sent, headers).await?;
    match answer.body {
        RecoveredBody::Completion(response) => Ok((
            ChatCompletion::OpenAi(Box::new(response.for_client(client_model))),
            answer.headers,
        )),
        RecoveredBody::Chunk(_) => Err(ChatFailure::Mismatched),
    }
}

// the two frames LiteLLM's stream wrapper sends for the one recovered chunk
pub(crate) async fn stream(
    calls: &ChatCalls,
    sent: &JsonValue,
    headers: HeaderMap,
    upstream_time: Duration,
) -> Result<ChatStreamAnswer, ChatFailure> {
    let (frames, headers) =
        recovered_frames(&calls.executor, &calls.counter, sent, headers).await?;
    Ok(ChatStreamAnswer {
        body: stream::iter(frames.into_iter().map(Ok)).boxed(),
        upstream_headers: headers,
        upstream_time,
    })
}

// the same two frames for a route that holds the executor and the token counter itself
pub(crate) async fn recovered_frames(
    executor: &ChatExecutor,
    tokens: &Counter,
    sent: &JsonValue,
    headers: HeaderMap,
) -> Result<([Vec<u8>; 2], HeaderMap), ChatFailure> {
    let answer = recover(executor, tokens, sent, headers).await?;
    let RecoveredBody::Chunk(chunk) = answer.body else {
        return Err(ChatFailure::Mismatched);
    };
    Ok((chunk_frames(chunk)?, answer.headers))
}

fn chunk_frames(mut chunk: Chunk) -> Result<[Vec<u8>; 2], ChatFailure> {
    let held: Vec<Option<String>> = chunk
        .choices
        .iter_mut()
        .map(|choice| choice.finish_reason.take())
        .collect();
    let first =
        sse_frame(&chunk).map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
    for (choice, reason) in chunk.choices.iter_mut().zip(held) {
        choice.finish_reason = reason;
        choice.delta = Delta::default();
    }
    let second =
        sse_frame(&chunk).map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
    Ok([first, second])
}

async fn recover(
    executor: &ChatExecutor,
    tokens: &Counter,
    sent: &JsonValue,
    headers: HeaderMap,
) -> Result<OutputLimitResponse, ChatFailure> {
    let encoded = serde_json::to_vec(sent)
        .map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
    // build_limit_response takes serde_json's own map, so this type comes from the call
    let data = serde_json::from_slice(&encoded)
        .map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
    let recovery = Recovery {
        clock: executor.clock,
        next_id: executor.next_id,
        tokens,
    };
    build_limit_response(&data, headers, &recovery)
        .await
        .map_err(|error| ChatFailure::Internal {
            message: error.to_string(),
        })
}

mod tests;
