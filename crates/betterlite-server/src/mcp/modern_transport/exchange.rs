use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{HeaderValue, Response, StatusCode, header};
use futures_channel::mpsc;
use futures_util::SinkExt as _;
use futures_util::StreamExt as _;
use futures_util::stream::Stream;
use rmcp::RoleServer;
use rmcp::ServerHandler;
use rmcp::model::{
    ClientJsonRpcMessage, ClientRequest, ErrorCode, ErrorData, JsonRpcError, JsonRpcMessage,
    JsonRpcRequest, JsonRpcResponse, RequestId, ServerResult,
};
use rmcp::service::{RunningService, TxJsonRpcMessage, serve_directly};
use tokio::time::{Interval, interval, timeout};

use crate::mcp::stream_transport::wire;

use super::{error_response, error_status, json_response};

const SERVICE_CHANNEL_CAPACITY: usize = 32;
// the SDK's _SSE_PING_INTERVAL
const PING_INTERVAL: Duration = Duration::from_secs(15);
const PING: &[u8] = b": ping\r\n\r\n";
const INTERNAL_ERROR_MESSAGE: &str = "Internal server error";

pub(super) async fn serve_request<H>(
    call: JsonRpcRequest<ClientRequest>,
    handler: H,
) -> Response<Body>
where
    H: ServerHandler,
{
    let request_id = call.id.clone();
    let (mut to_service, to_service_rx) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
    let (from_service, from_service_rx) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
    let running = serve_directly(handler, (from_service, to_service_rx), None);
    if to_service
        .send(ClientJsonRpcMessage::Request(call))
        .await
        .is_err()
    {
        return internal_failure(&request_id);
    }
    let mut outgoing = from_service_rx;
    match first_event(&mut outgoing).await {
        FirstEvent::Reply(Reply::Response(response)) => json_response(
            StatusCode::OK,
            serde_json::to_string(&response).unwrap_or_default(),
        ),
        FirstEvent::Reply(Reply::Error(error)) => {
            let payload = serde_json::to_string(&error).unwrap_or_default();
            json_response(error_status(error.error.code), payload)
        }
        FirstEvent::Notification(message) => sse_response(
            Opening::Message(message),
            outgoing,
            request_id,
            to_service,
            running,
        ),
        FirstEvent::Ended => internal_failure(&request_id),
        FirstEvent::TimedOut => {
            sse_response(Opening::Ping, outgoing, request_id, to_service, running)
        }
    }
}

enum FirstEvent {
    Reply(Reply),
    Notification(Box<TxJsonRpcMessage<RoleServer>>),
    Ended,
    TimedOut,
}

enum Reply {
    Response(Box<JsonRpcResponse<ServerResult>>),
    Error(JsonRpcError),
}

async fn first_event(outgoing: &mut mpsc::Receiver<TxJsonRpcMessage<RoleServer>>) -> FirstEvent {
    loop {
        match timeout(PING_INTERVAL, outgoing.next()).await {
            Ok(Some(JsonRpcMessage::Response(response))) => {
                return FirstEvent::Reply(Reply::Response(Box::new(response)));
            }
            Ok(Some(JsonRpcMessage::Error(error))) => {
                return FirstEvent::Reply(Reply::Error(error));
            }
            // rmcp refuses a server-initiated request on 2026-07-28 before it reaches this sink
            Ok(Some(JsonRpcMessage::Request(_))) => {}
            Ok(Some(message @ JsonRpcMessage::Notification(_))) => {
                return FirstEvent::Notification(Box::new(message));
            }
            Ok(None) => return FirstEvent::Ended,
            Err(_elapsed) => return FirstEvent::TimedOut,
        }
    }
}

enum Opening {
    Message(Box<TxJsonRpcMessage<RoleServer>>),
    Ping,
}

struct SseStream<H>
where
    H: ServerHandler,
{
    outgoing: mpsc::Receiver<TxJsonRpcMessage<RoleServer>>,
    ping: Interval,
    opening: Option<Opening>,
    request_id: RequestId,
    // the service quits when its input closes, so the sender lives as long as the stream
    _service: mpsc::Sender<ClientJsonRpcMessage>,
    // dropping the running service cancels its handler, which a closed response body must do
    _running: RunningService<RoleServer, H>,
    finished: bool,
}

impl<H> Stream for SseStream<H>
where
    H: ServerHandler,
{
    type Item = Result<Bytes, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        if let Some(opening) = this.opening.take() {
            return Poll::Ready(Some(Ok(match opening {
                Opening::Message(message) => wire::sse_event(&message),
                Opening::Ping => {
                    this.ping.reset();
                    Bytes::from_static(PING)
                }
            })));
        }
        loop {
            match Pin::new(&mut this.outgoing).poll_next(cx) {
                // rmcp refuses a server-initiated request on 2026-07-28 before it reaches this sink
                Poll::Ready(Some(JsonRpcMessage::Request(_))) => {}
                Poll::Ready(Some(message)) => {
                    this.finished = matches!(
                        message,
                        JsonRpcMessage::Response(_) | JsonRpcMessage::Error(_)
                    );
                    return Poll::Ready(Some(Ok(wire::sse_event(&message))));
                }
                Poll::Ready(None) => {
                    this.finished = true;
                    let error = internal_error(this.request_id.clone());
                    return Poll::Ready(Some(Ok(wire::sse_event(&error))));
                }
                Poll::Pending => {
                    return match this.ping.poll_tick(cx) {
                        Poll::Ready(_) => Poll::Ready(Some(Ok(Bytes::from_static(PING)))),
                        Poll::Pending => Poll::Pending,
                    };
                }
            }
        }
    }
}

fn sse_response<H>(
    opening: Opening,
    outgoing: mpsc::Receiver<TxJsonRpcMessage<RoleServer>>,
    request_id: RequestId,
    service: mpsc::Sender<ClientJsonRpcMessage>,
    running: RunningService<RoleServer, H>,
) -> Response<Body>
where
    H: ServerHandler,
{
    let stream = SseStream {
        outgoing,
        ping: ping_interval(),
        opening: Some(opening),
        request_id,
        _service: service,
        _running: running,
        finished: false,
    };
    let mut response = Response::new(Body::from_stream(stream));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    headers.insert(
        header::HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    response
}

fn ping_interval() -> Interval {
    let mut ping = interval(PING_INTERVAL);
    ping.reset();
    ping
}

fn internal_failure(request_id: &RequestId) -> Response<Body> {
    error_response(
        Some(request_id),
        ErrorCode::INTERNAL_ERROR,
        INTERNAL_ERROR_MESSAGE,
        None,
    )
}

fn internal_error(request_id: RequestId) -> TxJsonRpcMessage<RoleServer> {
    JsonRpcMessage::Error(JsonRpcError::new(
        Some(request_id),
        ErrorData::new(ErrorCode::INTERNAL_ERROR, INTERNAL_ERROR_MESSAGE, None),
    ))
}
