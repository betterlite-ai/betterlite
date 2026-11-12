use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};

use betterlite_config::sections::mock_response::UsageDefaults;
use betterlite_context::call_context::CallContext;
use betterlite_http::error::HttpError;
use betterlite_openai::chat::ChatRequest as ClientChatRequest;
use betterlite_openai::chat::MessageContent;
use betterlite_openai::stream::chunk::{Choice, Chunk, Delta};
use futures_util::stream::{BoxStream, Stream, StreamExt as _};

use crate::mock::stream::{StreamSpec, chunks, usage_chunk};

use super::error::DispatchError;
use super::log::CallOutcome;
use super::terminal::Terminal;
use super::{Dispatch, Entry};
use crate::operation::chat::{ChatCallInput, ChatRequest, ChatResult, ChatStreamAnswer};

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
    dispatch.evaluate_rules(rules_input(&request.request).as_deref(), model)?;
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

// LiteLLM concatenates the string contents of every message, and only when the first one carries content
fn rules_input(request: &ClientChatRequest) -> Option<String> {
    let first = request.messages.first()?;
    first.content.as_ref()?;
    let mut buffer = String::new();
    for message in &request.messages {
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
        finished: false,
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
    finished: bool,
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
                this.finished = true;
            }
            Poll::Ready(Some(Err(error))) => {
                let message = error.to_string();
                if let Some(terminal) = this.terminal.take() {
                    terminal.record(CallOutcome::Failed, None, Some(&message));
                }
                this.finished = true;
            }
            Poll::Ready(Some(Ok(_))) | Poll::Pending => {}
        }
        polled
    }
}

impl Drop for TerminalStream {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Some(terminal) = self.terminal.take() {
            terminal.record(CallOutcome::Cancelled, None, None);
        }
    }
}

mod tests;

// LiteLLM's stream wrapper keeps the role on the first delta and on choices after the first
pub fn mock_frames(
    spec: &StreamSpec<'_>,
    request: &ClientChatRequest,
    prompt_tokens: Option<u64>,
    usage: UsageDefaults,
) -> Result<Vec<Vec<u8>>, serde_json::Error> {
    let mut frames = Vec::new();
    for (position, mut chunk) in chunks(spec).enumerate() {
        if let Some(first) = chunk.choices.first_mut().filter(|_| position > 0) {
            first.delta.role = None;
        }
        // the wrapper drops a chunk with no choice, which is every content chunk when n is zero
        if chunk.choices.is_empty() {
            continue;
        }
        frames.push(sse_frame(&chunk)?);
    }
    frames.push(sse_frame(&Chunk {
        id: spec.id.to_owned(),
        object: String::from("chat.completion.chunk"),
        created: spec.created,
        model: spec.model.to_owned(),
        choices: vec![Choice {
            index: 0,
            delta: Delta::default(),
            finish_reason: Some(String::from("stop")),
            logprobs: None,
        }],
        usage: None,
        system_fingerprint: None,
        service_tier: None,
    })?);
    let with_usage = request
        .stream_options
        .as_ref()
        .is_some_and(|options| options.include_usage);
    if with_usage || prompt_tokens.is_some() {
        let count = prompt_tokens.unwrap_or(usage.prompt_tokens);
        frames.push(sse_frame(&usage_chunk(spec, count, usage))?);
    }
    frames.push(b"data: [DONE]\n\n".to_vec());
    Ok(frames)
}

pub fn sse_frame(chunk: &Chunk) -> Result<Vec<u8>, serde_json::Error> {
    let text = serde_json::to_string(chunk)?;
    Ok(format!("data: {text}\n\n").into_bytes())
}
