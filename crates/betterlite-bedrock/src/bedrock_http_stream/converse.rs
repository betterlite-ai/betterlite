use std::convert::Infallible;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use http::HeaderMap;

use crate::agentcore_stream::adapter::header_pairs;
use crate::agentcore_stream::transport::{StreamHandler, StreamHead};
use crate::bedrock_http_stream::error::{HttpStreamError, status_error, transform_failure};
use crate::bedrock_http_stream::fake::{FakeCall, buffered};
use crate::bedrock_http_stream::stream::{ChunkStream, FramedCall, OpenedStream, framed};
use crate::bedrock_http_stream::{
    Opening, SendFailure, StreamDeps, body_text, log_open, read_body, send,
};
use crate::bedrock_settings::ResponseBody;
use crate::converse_response::request::RawReply;
use crate::image_call::plan::LogContext;

pub struct ConverseCall<'call> {
    pub model: &'call str,
    pub api_base: &'call str,
    pub headers: HeaderMap,
    pub data: &'call str,
    pub messages: &'call Json,
    pub json_mode: Option<bool>,
    pub fake_stream: bool,
}

// No catch: a status failure carries the bytes repr of the read body, and other failures pass through as themselves.
pub async fn make_sync_call<'deps>(
    call: ConverseCall<'_>,
    deps: &StreamDeps<'deps>,
    log: &mut LogContext<'_, '_>,
) -> Result<OpenedStream<'deps>, HttpStreamError<Infallible>> {
    let handler: Arc<dyn StreamHandler> = deps
        .supplied
        .as_ref()
        .map_or_else(|| Arc::clone(&deps.default), Arc::clone);
    let mut head = send(
        handler.as_ref(),
        Opening {
            api_base: call.api_base,
            // The call keeps its headers for the failure and fake paths below.
            headers: call.headers.clone(),
            body: call.data.as_bytes().to_vec(),
            stream: !call.fake_stream,
        },
        deps.timeout,
    )
    .await
    .map_err(opening_failure)?;
    if head.status != 200 {
        return Err(bad_status(&call, &mut head).await?);
    }
    if call.fake_stream {
        return fake_opened(call, deps, head, log).await;
    }
    let opened = framed_opened(&call, deps, head);
    log_open(log.state, log.env, call.messages, call.data);
    Ok(opened)
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
    let error = status_error(
        head.status,
        bytes_text(&body),
        &head.headers,
        call.api_base,
        ResponseBody::Read(body_text(&body)),
    )?;
    Ok(HttpStreamError::Provider {
        status: head.status,
        error: Box::new(error),
    })
}

async fn fake_opened<'deps>(
    call: ConverseCall<'_>,
    deps: &StreamDeps<'deps>,
    mut head: StreamHead,
    log: &mut LogContext<'_, '_>,
) -> Result<OpenedStream<'deps>, HttpStreamError<Infallible>> {
    let body = read_body(&mut head.chunks)
        .await
        .map_err(HttpStreamError::Body)?;
    let headers = header_pairs(&head.headers);
    let stream = buffered(
        FakeCall {
            model: call.model,
            messages: call.messages,
            data: call.data,
            json_mode: call.json_mode,
            now: deps.now,
        },
        &RawReply {
            headers: &headers,
            body: &body,
        },
        log,
        deps.ids.as_ref(),
    )
    .map_err(transform_failure)?;
    log_open(log.state, log.env, call.messages, call.data);
    Ok(OpenedStream {
        chunks: ChunkStream::Fake(stream),
        headers: head.headers,
    })
}

fn framed_opened<'deps>(
    call: &ConverseCall<'_>,
    deps: &StreamDeps<'deps>,
    head: StreamHead,
) -> OpenedStream<'deps> {
    framed(
        FramedCall {
            model: call.model,
            json_mode: call.json_mode,
            provider: None,
            sync: false,
        },
        deps,
        head,
    )
}

// message=str(response.read()): Python shows the bytes as a b'...' literal and escapes quote, controls and non-ASCII.
fn bytes_text(body: &[u8]) -> String {
    let quote = if body.contains(&b'\'') && !body.contains(&b'"') {
        '"'
    } else {
        '\''
    };
    let mut shown = format!("b{quote}");
    for &byte in body {
        match byte {
            b'\\' => shown.push_str("\\\\"),
            b'\n' => shown.push_str("\\n"),
            b'\r' => shown.push_str("\\r"),
            b'\t' => shown.push_str("\\t"),
            _ if char::from(byte) == quote => {
                shown.push('\\');
                shown.push(char::from(byte));
            }
            0x20..=0x7e => shown.push(char::from(byte)),
            _ => shown.push_str(&format!("\\x{byte:02x}")),
        }
    }
    shown.push(quote);
    shown
}
