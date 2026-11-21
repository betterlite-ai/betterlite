use std::time::Duration;

use axum::http::HeaderMap;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::MockPlan;
use betterlite_core::dispatch::chat::mock_stream::mock_frames;
use betterlite_core::mock::chat::{ChatAnswer, answer};
use betterlite_core::mock::stream::StreamSpec;
use betterlite_core::operation::chat::{ChatFailure, ChatStreamAnswer};
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::chat::mock_failure;
use futures_util::StreamExt as _;
use futures_util::stream;

// the frames a mock answered call streams, built with the executor's own clock and id
pub(super) fn mock_stream(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &ChatRequest,
    plan: &MockPlan<'_>,
) -> Result<ChatStreamAnswer, ChatFailure> {
    let answer = answer(plan).map_err(|error| ChatFailure::Route(mock_failure(error)))?;
    let ChatAnswer::Content(content) = answer else {
        // LiteLLM answers a 500 attribute error here; a 400 names the misuse instead
        return Err(ChatFailure::Route(mock_failure(MockError::ObjectStream)));
    };
    let id = format!("chatcmpl-{}", (executor.next_id)());
    let spec = StreamSpec {
        content,
        choice_count: plan.choice_count,
        model: request.model.as_deref().unwrap_or(&deployment.model_group),
        id: &id,
        created: (executor.clock)(),
    };
    let frames = mock_frames(&spec, request, plan.prompt_tokens, executor.mock.usage)
        .map_err(|source| ChatFailure::Route(ChatError::Encode { source }))?;
    let body = stream::iter(frames.into_iter().map(Ok)).boxed();
    Ok(ChatStreamAnswer {
        body,
        upstream_headers: HeaderMap::new(),
        upstream_time: Duration::ZERO,
    })
}
