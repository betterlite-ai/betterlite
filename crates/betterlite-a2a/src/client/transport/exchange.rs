use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::stream::SseParser;
use futures_util::StreamExt as _;
use futures_util::stream::{BoxStream, iter};
use http::Method;
use http::header::{CONTENT_TYPE, HeaderMap};
use serde::Deserialize;
use url::Url;

use super::EventStream;
use crate::client::error::{ClientError, transport_error};
use crate::client::records::core::CoreStreamEvent;

type EventBuilder = fn(AgentJson) -> CoreStreamEvent;

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<AgentJson>,
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

const fn outbound(
    url: Url,
    headers: HeaderMap,
    body: Vec<u8>,
    deadline: Duration,
) -> OutboundRequest {
    OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(body),
        deadline,
    }
}

pub(super) async fn buffered_event(
    http: ProxiedClient,
    url: Url,
    headers: HeaderMap,
    request_body: Vec<u8>,
    deadline: Duration,
) -> Result<EventStream, ClientError> {
    buffered_result(http, url, headers, request_body, deadline)
        .await
        .map(|result| iter(send_event(&result).map(Ok)).boxed())
}
async fn buffered_result(
    http: ProxiedClient,
    url: Url,
    headers: HeaderMap,
    request_body: Vec<u8>,
    deadline: Duration,
) -> Result<AgentJson, ClientError> {
    let url_text = url.to_string();
    let request = outbound(url, headers, request_body, deadline);
    let response = http
        .send_buffered(request)
        .await
        .map_err(|source| transport_error(&url_text, deadline, source))?;
    check_status(response.status, &url_text)?;
    parse_rpc(&response.body, &url_text)
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
fn send_event(result: &AgentJson) -> Option<CoreStreamEvent> {
    if let Some(task) = result.get("task") {
        return Some(CoreStreamEvent::Task(task.clone()));
    }
    result.get("message").cloned().map(CoreStreamEvent::Message)
}
pub(super) async fn streaming_events(
    http: ProxiedClient,
    url: Url,
    headers: HeaderMap,
    request_body: Vec<u8>,
    deadline: Duration,
) -> Result<EventStream, ClientError> {
    let url_text = url.to_string();
    let request = outbound(url, headers, request_body, deadline);
    let response = http
        .send(request)
        .await
        .map_err(|source| transport_error(&url_text, deadline, source))?;
    check_status(response.response.status().as_u16(), &url_text)?;
    let content_type = response
        .response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if !content_type.contains("text/event-stream") {
        let buffered = response
            .buffered(http.limits.body_bytes)
            .await
            .map_err(|source| transport_error(&url_text, deadline, source))?;
        let event = stream_event(&buffered.body, &url_text)?;
        return Ok(iter(Some(Ok(event))).boxed());
    }
    Ok(sse_events(response.body_stream(), url_text, deadline))
}

fn sse_events(
    mut body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    url: String,
    deadline: Duration,
) -> EventStream {
    async_stream::stream! {
        let mut parser = SseParser::default();
        while let Some(chunk) = body.next().await {
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(source) => {
                    yield Err(read_error(&url, deadline, source));
                    return;
                }
            };
            let Ok(payloads) = parser.push(&bytes) else {
                yield Err(ClientError::EventTooLarge { url: url.clone() });
                return;
            };
            for payload in payloads {
                match stream_event(&payload, &url) {
                    Ok(event) => yield Ok(event),
                    Err(error) => {
                        yield Err(error);
                        return;
                    }
                }
            }
        }
        for payload in parser.finish() {
            match stream_event(&payload, &url) {
                Ok(event) => yield Ok(event),
                Err(error) => {
                    yield Err(error);
                    return;
                }
            }
        }
    }
    .boxed()
}
fn stream_event(payload: &[u8], url: &str) -> Result<CoreStreamEvent, ClientError> {
    let result = parse_rpc(payload, url)?;
    let events: [(&str, EventBuilder); 4] = [
        ("message", CoreStreamEvent::Message),
        ("task", CoreStreamEvent::Task),
        ("statusUpdate", CoreStreamEvent::StatusUpdate),
        ("artifactUpdate", CoreStreamEvent::ArtifactUpdate),
    ];
    for (key, build) in events {
        if let Some(found) = result.get(key) {
            return Ok(build(found.clone()));
        }
    }
    Err(ClientError::Protocol {
        url: url.to_owned(),
    })
}

fn parse_rpc(payload: &[u8], url: &str) -> Result<AgentJson, ClientError> {
    let response: JsonRpcResponse =
        serde_json::from_slice(payload).map_err(|source| ClientError::Decode {
            url: url.to_owned(),
            source,
        })?;
    if let Some(error) = response.error {
        return Err(ClientError::JsonRpc {
            url: url.to_owned(),
            code: error.code.unwrap_or_default(),
            message: error.message.unwrap_or_default(),
        });
    }
    response.result.ok_or_else(|| ClientError::Protocol {
        url: url.to_owned(),
    })
}

fn read_error(url: &str, deadline: Duration, failure: HttpError) -> ClientError {
    match failure {
        HttpError::Request(source) => ClientError::Connection {
            url: url.to_owned(),
            source: TransportError::Request(source),
        },
        HttpError::Timeout => ClientError::Timeout {
            url: url.to_owned(),
            timeout: deadline,
            source: None,
        },
        HttpError::TooLarge { .. } => ClientError::EventTooLarge {
            url: url.to_owned(),
        },
        other @ (HttpError::Required { .. }
        | HttpError::DotSegment { .. }
        | HttpError::UnsupportedProxyScheme { .. }
        | HttpError::InvalidProxyUrl { .. }
        | HttpError::Client(_)) => ClientError::Request {
            url: url.to_owned(),
            source: TransportError::Rejected(other),
        },
    }
}
