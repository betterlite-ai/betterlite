use std::convert::Infallible;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use http::HeaderMap;

use crate::agentcore_stream::adapter::{header_pairs, read_body};
use crate::agentcore_stream::transport::{StreamHandler, StreamHead};
use crate::bedrock_http_stream::error::{HttpStreamError, status_error, transform_failure};
use crate::bedrock_http_stream::fake::{FakeCall, buffered};
use crate::bedrock_http_stream::stream::{ChunkStream, FramedCall, OpenedStream, framed};
use crate::bedrock_http_stream::{
    Opening, PendingOpen, SendFailure, StreamDeps, body_text, log_open, send,
};
use crate::bedrock_settings::ResponseBody;
use crate::converse_response::request::RawReply;
use crate::event_frames::tally::repr_bytes;
use crate::image_call::plan::LogContext;
use crate::invoke_events::fake::FakeStream;

pub struct ConverseCall<'call> {
    pub model: &'call str,
    pub api_base: &'call str,
    pub headers: HeaderMap,
    pub data: &'call str,
    pub messages: &'call Json,
    pub json_mode: Option<bool>,
    pub fake_stream: bool,
}

// The POST, the 200 gate and the fake stream's body read; make_sync_call finishes the stream and logs it.
pub async fn open_call(
    call: &ConverseCall<'_>,
    deps: &StreamDeps<'_>,
) -> Result<PendingOpen, HttpStreamError<Infallible>> {
    let handler: Arc<dyn StreamHandler> = deps
        .supplied
        .as_ref()
        .map_or_else(|| Arc::clone(&deps.default), Arc::clone);
    let mut head = send(
        handler.as_ref(),
        Opening {
            api_base: call.api_base,
            // The call keeps its headers for the failure path below.
            headers: call.headers.clone(),
            body: call.data.as_bytes().to_vec(),
            stream: !call.fake_stream,
        },
        deps.timeout,
    )
    .await
    .map_err(opening_failure)?;
    if head.status != 200 {
        return Err(bad_status(call, &mut head).await?);
    }
    if call.fake_stream {
        let body = read_body(&mut head.chunks)
            .await
            .map_err(HttpStreamError::Body)?;
        return Ok(PendingOpen {
            head,
            body: Some(body),
        });
    }
    Ok(PendingOpen { head, body: None })
}

// No catch: a status failure carries the bytes repr of the read body, and other failures pass through as themselves.
pub fn make_sync_call<'deps>(
    call: &ConverseCall<'_>,
    deps: &StreamDeps<'deps>,
    pending: PendingOpen,
    log: &mut LogContext<'_, '_>,
) -> Result<OpenedStream<'deps>, HttpStreamError<Infallible>> {
    let PendingOpen { head, body } = pending;
    if let Some(body) = body {
        let stream = fake_stream(call, deps, &head, &body, log)?;
        log_open(log.state, log.env, call.messages, call.data);
        return Ok(OpenedStream {
            chunks: ChunkStream::Fake(stream),
            headers: head.headers,
        });
    }
    let opened = framed_opened(call, deps, head);
    log_open(log.state, log.env, call.messages, call.data);
    Ok(opened)
}

fn fake_stream(
    call: &ConverseCall<'_>,
    deps: &StreamDeps<'_>,
    head: &StreamHead,
    body: &[u8],
    log: &mut LogContext<'_, '_>,
) -> Result<FakeStream, HttpStreamError<Infallible>> {
    let headers = header_pairs(&head.headers);
    buffered(
        &FakeCall {
            model: call.model,
            messages: call.messages,
            data: call.data,
            json_mode: call.json_mode,
            now: deps.now,
        },
        &RawReply {
            headers: &headers,
            body,
        },
        log,
        deps.ids.as_ref(),
    )
    .map_err(transform_failure)
}

fn framed_opened<'deps>(
    call: &ConverseCall<'_>,
    deps: &StreamDeps<'deps>,
    head: StreamHead,
) -> OpenedStream<'deps> {
    framed(
        &FramedCall {
            model: call.model,
            json_mode: call.json_mode,
            provider: None,
            sync: false,
        },
        deps,
        head,
    )
}

fn opening_failure(failure: SendFailure) -> HttpStreamError<Infallible> {
    match failure {
        SendFailure::Url(source) => HttpStreamError::Url(source),
        SendFailure::Transport(source) => HttpStreamError::Transport(source),
    }
}

async fn bad_status(
    call: &ConverseCall<'_>,
    head: &mut StreamHead,
) -> Result<HttpStreamError<Infallible>, HttpStreamError<Infallible>> {
    let body = read_body(&mut head.chunks)
        .await
        .map_err(HttpStreamError::Body)?;
    // message=str(response.read()): Python shows the bytes as a b'...' literal and escapes quote, controls and non-ASCII.
    let error = status_error(
        head.status,
        repr_bytes(&body),
        &head.headers,
        call.api_base,
        ResponseBody::Read(body_text(&body)),
    )?;
    Ok(HttpStreamError::Provider {
        status: head.status,
        error: Box::new(error),
    })
}
