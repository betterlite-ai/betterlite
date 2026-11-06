use std::collections::VecDeque;
use std::io;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::HeaderMap;
use axum::response::{IntoResponse as _, Response};
use betterlite_config::sections::mock_response::UsageDefaults;
use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::{MockPlan, resolve_mock};
use betterlite_core::mock::chat::{ChatAnswer, answer};
use betterlite_core::mock::stream::{StreamSpec, chunks, usage_chunk};
use betterlite_http::error::HttpError;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::chat::mock_failure;
use betterlite_openai::stream::SseParser;
use betterlite_openai::stream::chunk::{Choice, Chunk, Delta};
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use super::headers::stream_headers;
use super::{CallContext, failure};

struct RelayState {
    upstream: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    parser: SseParser,
    pending: VecDeque<Bytes>,
    finished: bool,
    model: String,
    include_usage: bool,
}

pub(super) async fn stream_chat(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &ChatRequest,
    call: CallContext<'_>,
) -> Response {
    let mock = resolve_mock(
        executor.mock.responses.get(&deployment.model_id),
        executor.mock.permission,
        request.mock_response.as_ref(),
        request.mock_tool_calls.as_deref(),
        request.choice_count,
    );
    if let Some(plan) = mock {
        return mock_stream(executor, deployment, request, &plan, call);
    }
    let upstream_started = Instant::now();
    match executor.stream(deployment, request).await {
        Ok(upstream) => {
            let upstream_time = upstream_started.elapsed();
            let headers = stream_headers(
                upstream.response.headers(),
                deployment,
                call.id,
                upstream_time,
                call.started.elapsed(),
            );
            let stream_body = Body::from_stream(relay_bytes(upstream.body_stream(), request));
            (headers, stream_body).into_response()
        }
        Err(error) => failure(&error, call, Some(deployment)),
    }
}

fn mock_stream(
    executor: &ChatExecutor,
    deployment: &ChatDeployment,
    request: &ChatRequest,
    plan: &MockPlan<'_>,
    call: CallContext<'_>,
) -> Response {
    let answer = match answer(plan) {
        Ok(answer) => answer,
        Err(error) => return failure(&mock_failure(error), call, Some(deployment)),
    };
    let ChatAnswer::Content(content) = answer else {
        // LiteLLM answers a 500 attribute error here; a 400 names the misuse instead
        return failure(
            &mock_failure(MockError::ObjectStream),
            call,
            Some(deployment),
        );
    };
    let id = format!("chatcmpl-{}", (executor.next_id)());
    let spec = StreamSpec {
        content,
        choice_count: plan.choice_count,
        model: call.model.unwrap_or_default(),
        id: &id,
        created: (executor.clock)(),
    };
    let frames = match mock_frames(
        &spec,
        plan.prompt_tokens,
        executor.mock.usage,
        call.include_usage,
    ) {
        Ok(frames) => frames,
        Err(source) => return failure(&ChatError::Encode { source }, call, Some(deployment)),
    };
    let headers = stream_headers(
        &HeaderMap::new(),
        deployment,
        call.id,
        Duration::ZERO,
        call.started.elapsed(),
    );
    let body = stream::iter(frames.into_iter().map(Ok)).boxed();
    (headers, Body::from_stream(relay_bytes(body, request))).into_response()
}

// LiteLLM's stream wrapper keeps the role on the first delta and on choices after the first
fn mock_frames(
    spec: &StreamSpec<'_>,
    prompt_tokens: Option<u64>,
    usage: UsageDefaults,
    include_usage: bool,
) -> Result<Vec<Vec<u8>>, serde_json::Error> {
    let mut frames = Vec::new();
    for (position, mut chunk) in chunks(spec).enumerate() {
        if position > 0
            && let Some(first) = chunk.choices.first_mut()
        {
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
    if include_usage || prompt_tokens.is_some() {
        let count = prompt_tokens.unwrap_or(usage.prompt_tokens);
        frames.push(sse_frame(&usage_chunk(spec, count, usage))?);
    }
    frames.push(b"data: [DONE]\n\n".to_vec());
    Ok(frames)
}

fn sse_frame(chunk: &Chunk) -> Result<Vec<u8>, serde_json::Error> {
    let text = serde_json::to_string(chunk)?;
    Ok(format!("data: {text}\n\n").into_bytes())
}

fn relay_bytes(
    upstream: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    request: &ChatRequest,
) -> impl futures_util::Stream<Item = Result<Bytes, io::Error>> + use<> {
    let state = RelayState {
        upstream,
        parser: SseParser::default(),
        pending: VecDeque::new(),
        finished: false,
        model: request.model.clone().unwrap_or_default(),
        include_usage: request
            .stream_options
            .as_ref()
            .is_some_and(|options| options.include_usage),
    };
    stream::unfold(state, next_sse)
}

async fn next_sse(mut state: RelayState) -> Option<(Result<Bytes, io::Error>, RelayState)> {
    loop {
        if let Some(event) = state.pending.pop_front() {
            return Some((Ok(event), state));
        }
        if state.finished {
            return None;
        }
        let events = match state.upstream.next().await {
            Some(Ok(chunk)) => state.parser.push(&chunk).map_err(io::Error::other),
            Some(Err(error)) => Err(io::Error::other(error)),
            None => {
                state.finished = true;
                Ok(vec![b"[DONE]".to_vec()])
            }
        };
        if let Err(error) = events.and_then(|parsed| append_events(&mut state, parsed)) {
            state.finished = true;
            return Some((Err(error), state));
        }
    }
}

fn append_events(state: &mut RelayState, events: Vec<Vec<u8>>) -> Result<(), io::Error> {
    for event in events {
        if event == b"[DONE]" {
            state
                .pending
                .push_back(Bytes::from_static(b"data: [DONE]\n\n"));
            state.finished = true;
            break;
        }
        if let Some(chunk) =
            Chunk::parse(&event, &state.model, state.include_usage).map_err(io::Error::other)?
        {
            let text = serde_json::to_string(&chunk).map_err(io::Error::other)?;
            state
                .pending
                .push_back(Bytes::from(format!("data: {text}\n\n")));
        }
    }
    Ok(())
}

mod tests;
