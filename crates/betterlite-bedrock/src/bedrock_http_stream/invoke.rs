use std::convert::Infallible;
use std::sync::Arc;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::HeaderMap;

use crate::agentcore_stream::adapter::{header_pairs, read_body};
use crate::agentcore_stream::transport::{StreamHandler, StreamHead};
use crate::bedrock_http_stream::error::{
    HttpStreamError, raised, reason_phrase, status_error, transform_failure,
};
use crate::bedrock_http_stream::fake::{FakeCall, buffered};
use crate::bedrock_http_stream::stream::{ChunkStream, FramedCall, OpenedStream, framed};
use crate::bedrock_http_stream::{
    Opening, PendingOpen, SendFailure, StreamDeps, body_text, log_open, send,
};
use crate::bedrock_settings::ResponseBody;
use crate::converse_response::request::RawReply;
use crate::converse_response::transform::TransformError;
use crate::image_call::plan::LogContext;
use crate::invoke_events::fake::FakeStream;

pub(crate) const TIMEOUT_MESSAGE: &str = "Timeout error occurred.";

pub struct InvokeCall<'call> {
    pub model: &'call str,
    pub api_base: &'call str,
    pub headers: HeaderMap,
    pub data: &'call str,
    pub signed_json_body: Option<&'call [u8]>,
    pub messages: &'call Json,
    pub fake_stream: bool,
    pub json_mode: Option<bool>,
    pub provider: Option<&'call str>,
    pub sync: bool,
}

// The POST, the 200 gate and the fake stream's body read; make_call finishes the stream and logs it.
pub async fn open_call(
    call: &InvokeCall<'_>,
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
            body: request_body(call),
            stream: !call.fake_stream,
        },
        deps.timeout,
    )
    .await
    .map_err(opening_failure)?;
    if head.status != 200 {
        return Err(bad_status(call, &mut head).await);
    }
    if call.fake_stream {
        let body = read_body(&mut head.chunks)
            .await
            .map_err(|error| read_failure(&error))?;
        return Ok(PendingOpen {
            head,
            body: Some(body),
        });
    }
    Ok(PendingOpen { head, body: None })
}

// The opened iterator a caller consumes, after the buffered conversion and post_call LiteLLM runs before any read.
pub fn make_call<'deps>(
    call: &InvokeCall<'_>,
    deps: &StreamDeps<'deps>,
    pending: PendingOpen,
    log: &mut LogContext<'_, '_>,
) -> Result<OpenedStream<'deps>, HttpStreamError<Infallible>> {
    let PendingOpen { head, body } = pending;
    if let Some(buffered) = body {
        let stream = fake_stream(call, deps, &head, &buffered, log)?;
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
    call: &InvokeCall<'_>,
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
    .map_err(projection_failure)
}

fn framed_opened<'deps>(
    call: &InvokeCall<'_>,
    deps: &StreamDeps<'deps>,
    head: StreamHead,
) -> OpenedStream<'deps> {
    framed(
        &FramedCall {
            model: call.model,
            json_mode: call.json_mode,
            provider: call.provider,
            sync: call.sync,
        },
        deps,
        head,
    )
}

// make_sync_call sends the signed body whenever it has one, even empty bytes; make_call always sends data.
fn request_body(call: &InvokeCall<'_>) -> Vec<u8> {
    match (call.sync, call.signed_json_body) {
        (true, Some(signed)) => signed.to_vec(),
        (true, None) | (false, _) => call.data.as_bytes().to_vec(),
    }
}

// A fake stream had its body read, so the status failure carries the text; a streamed one carries the reason phrase unread.
async fn bad_status(call: &InvokeCall<'_>, head: &mut StreamHead) -> HttpStreamError<Infallible> {
    if !call.fake_stream {
        let message = reason_phrase(head.status);
        return status_failure(
            head.status,
            message.clone(),
            &head.headers,
            call.api_base,
            ResponseBody::Unread {
                reason_phrase: message,
            },
        );
    }
    match read_body(&mut head.chunks).await {
        Ok(body) => {
            let text = body_text(&body);
            status_failure(
                head.status,
                text.clone(),
                &head.headers,
                call.api_base,
                ResponseBody::Read(text),
            )
        }
        Err(error) => read_failure(&error),
    }
}

fn status_failure(
    status: u16,
    message: String,
    headers: &HeaderMap,
    url: &str,
    body: ResponseBody,
) -> HttpStreamError<Infallible> {
    match status_error(status, message, headers, url, body) {
        Ok(error) => HttpStreamError::Provider {
            status,
            error: Box::new(error),
        },
        Err(source) => HttpStreamError::Header(source),
    }
}

fn opening_failure(failure: SendFailure) -> HttpStreamError<Infallible> {
    match failure {
        SendFailure::Url(source) => raised(500, source.to_string()),
        SendFailure::Transport(source) if transport_timed_out(&source) => {
            raised(408, TIMEOUT_MESSAGE.to_owned())
        }
        SendFailure::Transport(source) => raised(500, source.to_string()),
    }
}

fn read_failure(error: &HttpError) -> HttpStreamError<Infallible> {
    if read_timed_out(error) {
        return raised(408, TIMEOUT_MESSAGE.to_owned());
    }
    raised(500, error.to_string())
}

// A BedrockError the converter raised keeps its status; every other conversion failure is the catch-all's 500.
fn projection_failure(error: TransformError<Infallible>) -> HttpStreamError<Infallible> {
    match transform_failure(error) {
        HttpStreamError::Transform(source) => raised(500, source.to_string()),
        passthrough @ (HttpStreamError::Provider { .. }
        | HttpStreamError::Transport(_)
        | HttpStreamError::Url(_)
        | HttpStreamError::Body(_)
        | HttpStreamError::Header(_)) => passthrough,
    }
}

const fn transport_timed_out(error: &TransportError) -> bool {
    matches!(
        error,
        TransportError::ConnectTimeout(_)
            | TransportError::Deadline(_)
            | TransportError::Rejected(HttpError::Timeout)
    )
}

fn read_timed_out(error: &HttpError) -> bool {
    matches!(error, HttpError::Timeout)
        || matches!(error, HttpError::Request(source) if source.is_timeout())
}
