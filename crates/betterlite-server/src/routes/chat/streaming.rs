use std::collections::VecDeque;
use std::io;

use axum::body::{Body, Bytes};
use axum::response::{IntoResponse as _, Response};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::StreamResponse;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::stream::SseParser;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use super::failure;
use super::headers::stream_headers;

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
    call_id: &str,
) -> Response {
    match executor.stream(deployment, request).await {
        Ok(upstream) => {
            let headers = stream_headers(&upstream, call_id);
            let stream_body = Body::from_stream(relay_sse(upstream, request));
            (headers, stream_body).into_response()
        }
        Err(error) => failure(&error, call_id, Some(deployment)),
    }
}

fn relay_sse(
    upstream: StreamResponse,
    request: &ChatRequest,
) -> impl futures_util::Stream<Item = Result<Bytes, io::Error>> + use<> {
    let state = RelayState {
        upstream: upstream.body_stream(),
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
