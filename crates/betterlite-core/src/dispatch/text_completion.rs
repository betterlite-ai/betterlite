use std::sync::Arc;

use betterlite_context::call_context::CallContext;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::completion_chat_bridge::CompletionEndpoint;
use betterlite_openai::text_completion_request::CompletionRequest;

use super::Dispatch;
use super::error::DispatchError;
use super::log::CallOutcome;
use crate::operation::Entry;
use crate::operation::text_completion::{
    ChatCallAnswer, ChatCallMode, TextCompletionCallInput, TextCompletionFailure,
    TextCompletionRequest, TextCompletionResult,
};

pub async fn execute<Metadata, Client>(
    request: TextCompletionRequest,
    context: &CallContext<'_, Metadata, Client>,
) -> Result<TextCompletionResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    let dispatch = Arc::clone(&request.dispatch);
    let terminal = dispatch.terminal(Entry::TextCompletion, &request.call_id, context);
    let result = complete(&dispatch, &request, context).await;
    let outcome = if result.is_ok() {
        CallOutcome::Succeeded
    } else {
        CallOutcome::Failed
    };
    terminal.record(outcome, result.as_ref().err(), None);
    result
}

async fn complete<Metadata, Client>(
    dispatch: &Dispatch,
    request: &TextCompletionRequest,
    context: &CallContext<'_, Metadata, Client>,
) -> Result<TextCompletionResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    let deployment = dispatch.bounded_deployment(&request.model_id, &request.call_id, context)?;
    let model = request
        .request
        .model
        .as_deref()
        .unwrap_or(&request.model_id);
    dispatch.evaluate_rules(rules_input(&request.request).as_deref(), model)?;
    endpoint_call(request, deployment).await
}

async fn endpoint_call(
    request: &TextCompletionRequest,
    deployment: ChatDeployment,
) -> Result<TextCompletionResult, DispatchError> {
    let endpoint = request.calls.endpoint(&deployment).await;
    let call = TextCompletionCallInput {
        deployment,
        request: &request.request,
    };
    match endpoint {
        CompletionEndpoint::Text => request.calls.complete(call).await.map_err(rejected),
        CompletionEndpoint::Chat => request
            .calls
            .chat(call, ChatCallMode::Buffered)
            .await
            .and_then(ChatCallAnswer::buffered)
            .map_err(rejected),
    }
}

// LiteLLM's function_setup builds the pre-call text from the messages it made out of the prompt
pub(super) fn rules_input(request: &CompletionRequest) -> Option<String> {
    let messages = request.prompt.messages()?;
    let mut buffer = String::new();
    for message in &messages {
        if let Some(content) = message.get("content").and_then(|content| content.as_str()) {
            buffer.push_str(content);
        }
    }
    Some(buffer)
}

// the shared Rejected variant types error_type as text, so a null type travels to the route as an empty string
pub(super) fn rejected(failure: TextCompletionFailure) -> DispatchError {
    DispatchError::Rejected {
        status: failure.status,
        error_type: failure.error_type.unwrap_or_default(),
        param: failure.param,
        code: failure.status.to_string(),
        message: failure.message,
    }
}

mod tests;
