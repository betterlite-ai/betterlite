use std::mem;
use std::str;
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, iter};
use http::header::CONTENT_TYPE;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::time::timeout;

use super::EventStream;
use crate::client::error::{ClientError, transport_error};
use crate::client::records::core::{CoreSendResponse, CoreStreamEnvelope, CoreStreamEvent};

const MAX_EVENT_BYTES: usize = 0x0010_0000;

type SseFrame = (Option<String>, Vec<u8>);

#[derive(Deserialize)]
struct JsonRpcResponse<T> {
    result: Option<T>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    message: Option<String>,
}

pub(super) async fn buffered_event(
    http: ProxiedClient,
    request: OutboundRequest,
    limit: Duration,
) -> Result<EventStream, ClientError> {
    buffered_result(http, request, limit)
        .await
        .map(|result| iter(send_event(result).map(Ok)).boxed())
}

async fn buffered_result(
    http: ProxiedClient,
    request: OutboundRequest,
    limit: Duration,
) -> Result<CoreSendResponse, ClientError> {
    let url_text = request.url.to_string();
    let response = timeout(limit, http.send_buffered(request))
        .await
        .map_err(|_elapsed| ClientError::Timeout {
            url: url_text.clone(),
            timeout: limit,
            source: None,
        })?
        .map_err(|source| transport_error(&url_text, limit, source))?;
    check_status(response.status, &url_text)?;
    parse_rpc(&response.body, &url_text)
}

fn send_event(response: CoreSendResponse) -> Option<CoreStreamEvent> {
    if let Some(task) = response.task {
        return Some(CoreStreamEvent::Task(task));
    }
    response.message.map(CoreStreamEvent::Message)
}

pub(super) async fn streaming_events(
    http: ProxiedClient,
    request: OutboundRequest,
    limit: Duration,
) -> Result<EventStream, ClientError> {
    let url_text = request.url.to_string();
    let response = timeout(limit, http.send(request))
        .await
        .map_err(|_elapsed| ClientError::Timeout {
            url: url_text.clone(),
            timeout: limit,
            source: None,
        })?
        .map_err(|source| transport_error(&url_text, limit, source))?;
    check_status(response.response.status().as_u16(), &url_text)?;
    let content_type = response
        .response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if content_type.contains("text/event-stream") {
        return Ok(sse_events(response.body_stream(), url_text, limit));
    }
    let buffered = timeout(limit, response.buffered(http.limits.body_bytes))
        .await
        .map_err(|_elapsed| ClientError::Timeout {
            url: url_text.clone(),
            timeout: limit,
            source: None,
        })?
        .map_err(|source| transport_error(&url_text, limit, source))?;
    Ok(iter(Some(stream_event(&buffered.body, &url_text))).boxed())
}

fn check_status(status: u16, url: &str) -> Result<(), ClientError> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    Err(ClientError::Status {
        url: url.to_owned(),
        status,
    })
}

#[derive(Default)]
struct EventFrames {
    buffer: Vec<u8>,
}

impl EventFrames {
    #[rustfmt::skip]
    fn push(&mut self, bytes: &[u8], url: &str) -> Result<Vec<SseFrame>, ClientError> {
        if self.buffer.len().saturating_add(bytes.len()) > MAX_EVENT_BYTES {
            return Err(ClientError::EventTooLarge { url: url.to_owned() });
        }
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        while let Some(end) = blank_line(&self.buffer) {
            let frame: Vec<u8> = self.buffer.drain(..end).collect();
            if let Some(parts) = frame_parts(&frame) { frames.push(parts); }
        }
        Ok(frames)
    }

    fn finish(&mut self) -> Option<SseFrame> {
        if self.buffer.is_empty() {
            return None;
        }
        let frame = mem::take(&mut self.buffer);
        frame_parts(&frame)
    }
}

#[rustfmt::skip]
fn blank_line(buffer: &[u8]) -> Option<usize> {
    let lf = buffer.windows(2).position(|pair| pair == b"\n\n").and_then(|at| at.checked_add(2));
    let crlf = buffer.windows(4).position(|pair| pair == b"\r\n\r\n").and_then(|at| at.checked_add(4));
    [lf, crlf].into_iter().flatten().min()
}

#[rustfmt::skip]
fn frame_parts(frame: &[u8]) -> Option<SseFrame> {
    let mut event = None;
    let mut data = Vec::new();
    for line in frame.split(|byte| *byte == b'\n') {
        let trimmed = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some(name) = trimmed.strip_prefix(b"event:") { event = str::from_utf8(name).ok().map(|value| value.trim().to_owned()); continue; }
        let Some(value) = trimmed.strip_prefix(b"data:") else { continue; };
        if !data.is_empty() { data.push(b'\n'); }
        data.extend_from_slice(value.strip_prefix(b" ").unwrap_or(value));
    }
    (!data.is_empty()).then_some((event, data))
}

fn frame_event(frame: SseFrame, url: &str) -> Result<CoreStreamEvent, ClientError> {
    let (event, payload) = frame;
    if event.as_deref() == Some("error") {
        return Err(ClientError::SseError {
            message: String::from_utf8(payload).unwrap_or_default(),
        });
    }
    stream_event(&payload, url)
}

#[rustfmt::skip]
fn sse_events(mut body: BoxStream<'static, Result<Vec<u8>, HttpError>>, url: String, limit: Duration) -> EventStream {
    async_stream::stream! {
        let mut parser = EventFrames::default();
        loop {
            let bytes = match timeout(limit, body.next()).await {
                Ok(Some(Ok(bytes))) => bytes,
                Ok(Some(Err(source))) => { yield Err(read_error(&url, limit, source)); return; }
                Ok(None) => break,
                Err(_elapsed) => { yield Err(ClientError::Timeout { url: url.clone(), timeout: limit, source: None }); return; }
            };
            let frames = match parser.push(&bytes, &url) {
                Ok(frames) => frames,
                Err(error) => { yield Err(error); return; }
            };
            for frame in frames {
                match frame_event(frame, &url) {
                    Ok(event @ CoreStreamEvent::Message(_)) => { yield Ok(event); return; }
                    Ok(event) => yield Ok(event),
                    Err(error) => { yield Err(error); return; }
                }
            }
        }
        if let Some(frame) = parser.finish() {
            match frame_event(frame, &url) { Ok(event) => yield Ok(event), Err(error) => yield Err(error) }
        }
    }
    .boxed()
}

#[rustfmt::skip]
fn stream_event(payload: &[u8], url: &str) -> Result<CoreStreamEvent, ClientError> {
    let envelope: CoreStreamEnvelope = parse_rpc(payload, url)?;
    if let Some(message) = envelope.message { return Ok(CoreStreamEvent::Message(message)); }
    if let Some(task) = envelope.task { return Ok(CoreStreamEvent::Task(task)); }
    if let Some(update) = envelope.status_update { return Ok(CoreStreamEvent::StatusUpdate(update)); }
    if let Some(update) = envelope.artifact_update { return Ok(CoreStreamEvent::ArtifactUpdate(update)); }
    Err(ClientError::Protocol { url: url.to_owned() })
}

#[rustfmt::skip]
fn parse_rpc<T>(payload: &[u8], url: &str) -> Result<T, ClientError> where T: DeserializeOwned {
    let response: JsonRpcResponse<T> = serde_json::from_slice(payload).map_err(|source| ClientError::Decode { url: url.to_owned(), source })?;
    if let Some(error) = response.error {
        return Err(ClientError::JsonRpc { url: url.to_owned(), code: error.code.unwrap_or_default(), message: error.message.unwrap_or_default() });
    }
    response.result.ok_or_else(|| ClientError::Protocol { url: url.to_owned() })
}

#[rustfmt::skip]
fn read_error(url: &str, limit: Duration, failure: HttpError) -> ClientError {
    match failure {
        HttpError::Request(source) => {
            if source.is_timeout() {
                return ClientError::Timeout { url: url.to_owned(), timeout: limit, source: Some(TransportError::Deadline(source)) };
            }
            ClientError::Connection { url: url.to_owned(), source: TransportError::Request(source) }
        }
        HttpError::Timeout => ClientError::Timeout { url: url.to_owned(), timeout: limit, source: None },
        HttpError::TooLarge { .. } => ClientError::EventTooLarge { url: url.to_owned() },
        other @ (HttpError::Required { .. } | HttpError::DotSegment { .. } | HttpError::UnsupportedProxyScheme { .. } | HttpError::InvalidProxyUrl { .. } | HttpError::Client(_)) => ClientError::Request { url: url.to_owned(), source: TransportError::Rejected(other) },
    }
}
