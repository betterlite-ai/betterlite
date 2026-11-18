use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use betterlite_context::call_context::CallContext;
use betterlite_models::decisions::Json;
use betterlite_openai::completion_chat_bridge::CompletionEndpoint;
use betterlite_openai::text_completion_request::CompletionRequest;
use futures_util::stream::{BoxStream, Stream, StreamExt as _};
use indexmap::IndexMap;

use super::Dispatch;
use super::error::DispatchError;
use super::log::CallOutcome;
use super::terminal::Terminal;
use super::text_completion::{rejected, rules_input};
use crate::message_utilities::response_streams::text_completion::TextCompletionStreamWrapper;
use crate::operation::Entry;
use crate::operation::text_completion::{
    ChatCallAnswer, ChatCallMode, TextCompletionCallInput, TextCompletionFailure,
    TextCompletionRequest,
};
use crate::operation::text_stream::{TextStreamAnswer, TextStreamResult};

pub async fn execute<Metadata, Client>(
    request: TextCompletionRequest,
    context: &CallContext<'_, Metadata, Client>,
) -> Result<TextStreamResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    let dispatch = Arc::clone(&request.dispatch);
    let terminal = dispatch.terminal(Entry::TextStream, &request.call_id, context);
    let options = stream_options(&request.request);
    match stream(&dispatch, &request, context).await {
        Ok(answer) => Ok(with_terminal(answer, terminal, options)),
        Err(error) => {
            terminal.record(CallOutcome::Failed, Some(&error), None);
            Err(error)
        }
    }
}

async fn stream<Metadata, Client>(
    dispatch: &Dispatch,
    request: &TextCompletionRequest,
    context: &CallContext<'_, Metadata, Client>,
) -> Result<TextStreamAnswer, DispatchError>
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
    let endpoint = request.calls.endpoint(&deployment).await;
    let call = TextCompletionCallInput {
        deployment,
        request: &request.request,
    };
    // a chat-catalog deployment streams from the chat endpoint, and the wrapper converts its chunks
    match endpoint {
        CompletionEndpoint::Text => request.calls.stream(call).await.map_err(rejected),
        CompletionEndpoint::Chat => request
            .calls
            .chat(call, ChatCallMode::Streamed)
            .await
            .and_then(ChatCallAnswer::streamed)
            .map_err(rejected),
    }
}

fn stream_options(request: &CompletionRequest) -> Option<Json> {
    request.stream_options.as_ref().map(|options| {
        Json::Object(IndexMap::from([(
            String::from("include_usage"),
            Json::Bool(options.include_usage),
        )]))
    })
}

// the text completion wrapper B5366 owns reads chat chunks; a failed chunk is the wrapper's missing chunk
fn with_terminal(
    answer: TextStreamAnswer,
    terminal: Terminal,
    options: Option<Json>,
) -> TextStreamResult {
    let chunks = TerminalTextStream {
        inner: answer.body,
        terminal: Some(terminal),
    }
    .boxed()
    .map(|item| match item {
        Ok(chunk) => chunk,
        Err(failure) => {
            tracing::warn!(message = %failure.message, "the provider text stream failed");
            Json::Null
        }
    });
    let body = TextCompletionStreamWrapper {
        chunks,
        stream_options: options,
    }
    .boxed();
    TextStreamResult {
        body,
        upstream_headers: answer.upstream_headers,
        upstream_time: answer.upstream_time,
    }
}

struct TerminalTextStream {
    inner: BoxStream<'static, Result<Json, TextCompletionFailure>>,
    terminal: Option<Terminal>,
}

impl Stream for TerminalTextStream {
    type Item = Result<Json, TextCompletionFailure>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let polled = this.inner.as_mut().poll_next(cx);
        match &polled {
            Poll::Ready(None) => {
                if let Some(terminal) = this.terminal.take() {
                    terminal.record(CallOutcome::Succeeded, None, None);
                }
            }
            Poll::Ready(Some(Err(failure))) => {
                if let Some(terminal) = this.terminal.take() {
                    terminal.record(CallOutcome::Failed, None, Some(&failure.message));
                }
            }
            Poll::Ready(Some(Ok(_))) | Poll::Pending => {}
        }
        polled
    }
}

impl Drop for TerminalTextStream {
    fn drop(&mut self) {
        if let Some(terminal) = self.terminal.take() {
            terminal.record(CallOutcome::Cancelled, None, None);
        }
    }
}

mod tests;
