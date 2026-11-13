use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};

use betterlite_context::call_context::CallContext;
use betterlite_http::error::HttpError;
use betterlite_openai::chat::MessageContent;
use futures_util::stream::{BoxStream, Stream, StreamExt as _};

use super::error::DispatchError;
use super::log::CallOutcome;
use super::terminal::Terminal;
use super::{Dispatch, Entry};
use crate::operation::chat::{ChatCallInput, ChatRequest, ChatResult, ChatStreamAnswer};

pub mod mock_stream;

pub async fn execute<Metadata, Client>(
    dispatch: &Dispatch,
    request: ChatRequest,
    context: &CallContext<'_, Metadata, Client>,
) -> Result<ChatResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    let terminal = dispatch.terminal(Entry::Chat, &request.call_id, context);
    match run(dispatch, &request, context).await {
        Ok(ChatResult::Completion(answer)) => {
            terminal.record(CallOutcome::Succeeded, None, None);
            Ok(ChatResult::Completion(answer))
        }
        Ok(ChatResult::Stream(answer)) => Ok(ChatResult::Stream(with_terminal(answer, terminal))),
        Err(error) => {
            terminal.record(CallOutcome::Failed, Some(&error), None);
            Err(error)
        }
    }
}

async fn run<Metadata, Client>(
    dispatch: &Dispatch,
    request: &ChatRequest,
    context: &CallContext<'_, Metadata, Client>,
) -> Result<ChatResult, DispatchError>
where
    Metadata: Sync,
    Client: Sync,
{
    // supported settings and the call's own deadline decide the deployment timeout
    let deployment = dispatch.bounded_deployment(&request.model_id, &request.call_id, context)?;
    // pre-call rules read the message buffer LiteLLM's function_setup builds for chat calls
    let model = request
        .request
        .model
        .as_deref()
        .unwrap_or(&request.model_id);
    dispatch.evaluate_rules(rules_input(request).as_deref(), model)?;
    // parameter preparation parses the control options before any upstream request is built
    let control = request.calls.prepare(&request.request)?;
    let call = ChatCallInput {
        deployment,
        request: &request.request,
    };
    if request.request.stream {
        let answer = request.calls.stream(call, control).await?;
        Ok(ChatResult::Stream(answer))
    } else {
        let answer = request.calls.complete(call, control).await?;
        Ok(ChatResult::Completion(answer))
    }
}

// LiteLLM concatenates the string contents of every message once the first one has a content key
fn rules_input(request: &ChatRequest) -> Option<String> {
    if !request.first_message_has_content {
        return None;
    }
    let mut buffer = String::new();
    for message in &request.request.messages {
        if let Some(MessageContent::Text(text)) = &message.content {
            buffer.push_str(text);
        }
    }
    Some(buffer)
}

fn with_terminal(answer: ChatStreamAnswer, terminal: Terminal) -> ChatStreamAnswer {
    let body = TerminalStream {
        inner: answer.body,
        terminal: Some(terminal),
    }
    .boxed();
    ChatStreamAnswer {
        body,
        upstream_headers: answer.upstream_headers,
        upstream_time: answer.upstream_time,
    }
}

struct TerminalStream {
    inner: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    terminal: Option<Terminal>,
}

impl Stream for TerminalStream {
    type Item = Result<Vec<u8>, HttpError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let polled = this.inner.as_mut().poll_next(cx);
        match &polled {
            Poll::Ready(None) => {
                if let Some(terminal) = this.terminal.take() {
                    terminal.record(CallOutcome::Succeeded, None, None);
                }
            }
            Poll::Ready(Some(Err(error))) => {
                let message = error.to_string();
                if let Some(terminal) = this.terminal.take() {
                    terminal.record(CallOutcome::Failed, None, Some(&message));
                }
            }
            Poll::Ready(Some(Ok(_))) | Poll::Pending => {}
        }
        polled
    }
}

impl Drop for TerminalStream {
    fn drop(&mut self) {
        // a stream that already ended or failed has used its terminal
        if let Some(terminal) = self.terminal.take() {
            terminal.record(CallOutcome::Cancelled, None, None);
        }
    }
}

mod tests;
