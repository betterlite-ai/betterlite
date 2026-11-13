use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use axum::http::{Method, Request, Response, StatusCode};
use futures_channel::mpsc;
use futures_util::SinkExt as _;
use futures_util::stream::Stream;
use rmcp::model::{
    ClientJsonRpcMessage, ClientRequest, ErrorCode, GetMeta as _, JsonRpcRequest, ProgressToken,
    RequestId,
};
use rmcp::service::{RunningService, TxJsonRpcMessage, serve_directly};
use rmcp::{RoleServer, ServerHandler};
use tokio::time::{Interval, interval};

use super::session::{SERVICE_CHANNEL_CAPACITY, ping_interval, routed_request, terminal_request};
use super::wire;

pub(super) async fn handle<H>(request: Request<Body>, handler: H) -> Response<Body>
where
    H: ServerHandler,
{
    if *request.method() == Method::POST {
        return handle_post(request, handler).await;
    }
    if *request.method() == Method::GET {
        return handle_get(&request);
    }
    if *request.method() == Method::DELETE {
        return delete_refusal();
    }
    wire::method_not_allowed(None)
}

fn delete_refusal() -> Response<Body> {
    wire::error_response(
        StatusCode::METHOD_NOT_ALLOWED,
        ErrorCode::INVALID_REQUEST,
        "Method Not Allowed: Session termination not supported",
        None,
    )
}

async fn handle_post<H>(request: Request<Body>, handler: H) -> Response<Body>
where
    H: ServerHandler,
{
    let message = match wire::decode_post(request, None).await {
        Ok(message) => message,
        Err(response) => return *response,
    };
    match message {
        ClientJsonRpcMessage::Request(call) => serve_request(call, handler).await,
        other @ (ClientJsonRpcMessage::Response(_)
        | ClientJsonRpcMessage::Notification(_)
        | ClientJsonRpcMessage::Error(_)) => {
            dispatch_notification(other, handler).await;
            wire::json_body(StatusCode::ACCEPTED, Body::empty(), None)
        }
    }
}

fn handle_get(request: &Request<Body>) -> Response<Body> {
    let (_, has_sse) = wire::accepted_media_types(request.headers());
    if !has_sse {
        return wire::error_response(
            StatusCode::NOT_ACCEPTABLE,
            ErrorCode::INVALID_REQUEST,
            "Not Acceptable: Client must accept text/event-stream",
            None,
        );
    }
    let mut ping = interval(wire::PING_INTERVAL);
    ping.reset();
    wire::sse_response(Body::from_stream(PingStream(ping)), None)
}

struct PingStream(Interval);

impl Stream for PingStream {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.get_mut().0.poll_tick(cx) {
            Poll::Ready(_) => Poll::Ready(Some(Ok(wire::ping()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

async fn serve_request<H>(call: JsonRpcRequest<ClientRequest>, handler: H) -> Response<Body>
where
    H: ServerHandler,
{
    let request_id = call.id.clone();
    let progress_token = call.request.get_meta().get_progress_token();
    let (mut to_service, to_service_rx) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
    let (from_service, from_service_rx) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
    let running = serve_directly(handler, (from_service, to_service_rx), None);
    if to_service
        .send(ClientJsonRpcMessage::Request(call))
        .await
        .is_err()
    {
        return wire::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::INTERNAL_ERROR,
            "Error handling POST request",
            None,
        );
    }
    let mut progress_tokens = HashMap::new();
    if let Some(token) = progress_token {
        progress_tokens.insert(token, request_id.clone());
    }
    wire::sse_response(
        Body::from_stream(StatelessStream {
            outgoing: from_service_rx,
            ping: ping_interval(),
            request_id,
            progress_tokens,
            // the service quits when its input closes, so the sender lives as long as the stream
            _service: to_service,
            _running: running,
            finished: false,
        }),
        None,
    )
}

async fn dispatch_notification<H>(message: ClientJsonRpcMessage, handler: H)
where
    H: ServerHandler,
{
    let (mut to_service, to_service_rx) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
    let (from_service, from_service_rx) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
    let running = serve_directly(handler, (from_service, to_service_rx), None);
    let _sent = to_service.send(message).await;
    drop(from_service_rx);
    drop(running);
}

struct StatelessStream<H>
where
    H: ServerHandler,
{
    outgoing: mpsc::Receiver<TxJsonRpcMessage<RoleServer>>,
    ping: Interval,
    request_id: RequestId,
    progress_tokens: HashMap<ProgressToken, RequestId>,
    _service: mpsc::Sender<ClientJsonRpcMessage>,
    // dropping the running service cancels it, which an ended or disconnected stream must do
    _running: RunningService<RoleServer, H>,
    finished: bool,
}

enum NextEvent {
    Chunk(Bytes),
    Skip,
    Pending,
    End,
}

impl<H> Stream for StatelessStream<H>
where
    H: ServerHandler,
{
    type Item = Result<Bytes, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match next_event(this, cx) {
                NextEvent::Chunk(bytes) => return Poll::Ready(Some(Ok(bytes))),
                NextEvent::Skip => {}
                NextEvent::Pending => return Poll::Pending,
                NextEvent::End => return Poll::Ready(None),
            }
        }
    }
}

fn next_event<H>(state: &mut StatelessStream<H>, cx: &mut Context<'_>) -> NextEvent
where
    H: ServerHandler,
{
    if state.finished {
        return NextEvent::End;
    }
    let message = match Pin::new(&mut state.outgoing).poll_next(cx) {
        Poll::Ready(None) => return NextEvent::End,
        Poll::Ready(Some(message)) => message,
        Poll::Pending => {
            return match state.ping.poll_tick(cx) {
                Poll::Ready(_) => NextEvent::Chunk(wire::ping()),
                Poll::Pending => NextEvent::Pending,
            };
        }
    };
    if routed_request(&message, &state.progress_tokens).as_ref() != Some(&state.request_id) {
        return NextEvent::Skip;
    }
    state.finished = terminal_request(&message) == Some(&state.request_id);
    NextEvent::Chunk(wire::sse_event(&message))
}

mod tests;
