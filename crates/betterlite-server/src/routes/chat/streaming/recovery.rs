use std::time::Duration;

use axum::http::HeaderMap;
use betterlite_core::dispatch::chat::mock_stream::sse_frame;
use betterlite_core::operation::chat::{ChatFailure, ChatStreamAnswer};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::execution::ChatError;
use betterlite_openai::output_limits::build::{
    OutputLimitResponse, RecoveredBody, Recovery, build_limit_response,
};
use betterlite_openai::stream::chunk::Delta;
use betterlite_provider::operation::ChatCompletion;
use futures_util::StreamExt as _;
use futures_util::stream;

use super::ChatCalls;

// the buffered answer for an output-limit 400: the completion restamped with the model the client asked for
pub(super) async fn completion(
    calls: &ChatCalls,
    sent: &JsonValue,
    headers: HeaderMap,
    client_model: Option<&str>,
) -> Result<(ChatCompletion, HeaderMap), ChatFailure> {
    let answer = recover(calls, sent, headers).await?;
    let RecoveredBody::Completion(response) = answer.body else {
        return Err(ChatFailure::Mismatched);
    };
    Ok((
        ChatCompletion::OpenAi(Box::new(response.for_client(client_model))),
        answer.headers,
    ))
}

// the two frames LiteLLM's stream wrapper sends for the one recovered chunk
pub(super) async fn stream(
    calls: &ChatCalls,
    sent: &JsonValue,
    headers: HeaderMap,
    upstream_time: Duration,
) -> Result<ChatStreamAnswer, ChatFailure> {
    let answer = recover(calls, sent, headers).await?;
    let RecoveredBody::Chunk(mut chunk) = answer.body else {
        return Err(ChatFailure::Mismatched);
    };
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
    Ok(ChatStreamAnswer {
        body: stream::iter(vec![Ok(first), Ok(second)]).boxed(),
        upstream_headers: answer.headers,
        upstream_time,
    })
}

async fn recover(
    calls: &ChatCalls,
    sent: &JsonValue,
    headers: HeaderMap,
) -> Result<OutputLimitResponse, ChatFailure> {
    let encoded = serde_json::to_vec(sent)
        .map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
    // build_limit_response takes serde_json's own map, so this type comes from the call
    let data = serde_json::from_slice(&encoded)
        .map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
    let recovery = Recovery {
        clock: calls.executor.clock,
        next_id: calls.executor.next_id,
        tokens: &calls.counter,
    };
    build_limit_response(&data, headers, &recovery)
        .await
        .map_err(|error| ChatFailure::Internal {
            message: error.to_string(),
        })
}

mod tests;
