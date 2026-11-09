use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode};
use futures_channel::mpsc;
use futures_util::SinkExt as _;
use indexmap::IndexMap;
use rmcp::ServerHandler;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest, ErrorCode, GetMeta as _, JsonRpcRequest};

use super::session::{Session, SessionStreams, StreamKey, insert_stream, stream_state};
use super::wire;

struct SessionHandle {
    id: String,
    streams: Arc<Mutex<SessionStreams>>,
    service: mpsc::Sender<ClientJsonRpcMessage>,
}

pub(super) async fn handle<H>(
    sessions: &Mutex<IndexMap<String, Session>>,
    request: Request<Body>,
    handler: H,
) -> Response<Body>
where
    H: ServerHandler,
{
    let session_id = request
        .headers()
        .get(wire::MCP_SESSION_ID_HEADER)
        .map(|value| value.to_str().map(str::to_owned));
    match session_id {
        None => open_session(sessions, request, handler).await,
        Some(Ok(id)) => match lookup(sessions, &id) {
            Some(target) => serve_existing(sessions, target, request, handler).await,
            None => unknown_session(),
        },
        Some(Err(_)) => unknown_session(),
    }
}

fn lookup(sessions: &Mutex<IndexMap<String, Session>>, id: &str) -> Option<SessionHandle> {
    let map = sessions.lock().unwrap_or_else(PoisonError::into_inner);
    let entry = map.get(id)?;
    let service = entry.service.clone()?;
    let streams = Arc::clone(&entry.streams);
    drop(map);
    Some(SessionHandle {
        id: id.to_owned(),
        streams,
        service,
    })
}

fn unknown_session() -> Response<Body> {
    wire::error_response(
        StatusCode::NOT_FOUND,
        ErrorCode::INVALID_REQUEST,
        "Session not found",
        None,
    )
}

async fn open_session<H>(
    sessions: &Mutex<IndexMap<String, Session>>,
    request: Request<Body>,
    handler: H,
) -> Response<Body>
where
    H: ServerHandler,
{
    let id = uuid::Uuid::new_v4().simple().to_string();
    let mut session = Session::new();
    let response = match *request.method() {
        Method::POST => open_session_post(&mut session, &id, request, handler).await,
        Method::GET => new_session_get(&id, &request),
        Method::DELETE => missing_session_id(&id),
        _ => wire::method_not_allowed(Some(&id)),
    };
    if response.status().as_u16() < 400 {
        sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, session);
    }
    response
}

fn new_session_get(id: &str, request: &Request<Body>) -> Response<Body> {
    let (_, has_sse) = wire::accepted_media_types(request.headers());
    if !has_sse {
        return wire::error_response(
            StatusCode::NOT_ACCEPTABLE,
            ErrorCode::INVALID_REQUEST,
            "Not Acceptable: Client must accept text/event-stream",
            Some(id),
        );
    }
    missing_session_id(id)
}

fn missing_session_id(id: &str) -> Response<Body> {
    wire::error_response(
        StatusCode::BAD_REQUEST,
        ErrorCode::INVALID_REQUEST,
        "Bad Request: Missing session ID",
        Some(id),
    )
}

async fn open_session_post<H>(
    session: &mut Session,
    id: &str,
    request: Request<Body>,
    handler: H,
) -> Response<Body>
where
    H: ServerHandler,
{
    let message = match wire::decode_post(request, Some(id)).await {
        Ok(message) => message,
        Err(response) => return *response,
    };
    let ClientJsonRpcMessage::Request(call) = message else {
        return missing_session_id(id);
    };
    if call.request.method() != "initialize" {
        return missing_session_id(id);
    }
    session.start(handler);
    let Some(service) = session.service.clone() else {
        return wire::error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::INTERNAL_ERROR,
            "Error handling POST request",
            Some(id),
        );
    };
    let streams = Arc::clone(&session.streams);
    serve_request(&streams, id, service, call).await
}

async fn serve_existing<H>(
    sessions: &Mutex<IndexMap<String, Session>>,
    target: SessionHandle,
    request: Request<Body>,
    _handler: H,
) -> Response<Body>
where
    H: ServerHandler,
{
    match *request.method() {
        Method::POST => existing_post(target, request).await,
        Method::GET => existing_get(&target, &request),
        Method::DELETE => delete_session(sessions, &target),
        _ => wire::method_not_allowed(Some(&target.id)),
    }
}

async fn existing_post(target: SessionHandle, request: Request<Body>) -> Response<Body> {
    let message = match wire::decode_post(request, Some(&target.id)).await {
        Ok(message) => message,
        Err(response) => return *response,
    };
    let mut service = target.service;
    match message {
        ClientJsonRpcMessage::Request(call) => {
            serve_request(&target.streams, &target.id, service, call).await
        }
        other @ (ClientJsonRpcMessage::Response(_)
        | ClientJsonRpcMessage::Notification(_)
        | ClientJsonRpcMessage::Error(_)) => accept_message(&mut service, other, &target.id).await,
    }
}

fn existing_get(target: &SessionHandle, request: &Request<Body>) -> Response<Body> {
    let (_, has_sse) = wire::accepted_media_types(request.headers());
    if !has_sse {
        return wire::error_response(
            StatusCode::NOT_ACCEPTABLE,
            ErrorCode::INVALID_REQUEST,
            "Not Acceptable: Client must accept text/event-stream",
            Some(&target.id),
        );
    }
    let key = StreamKey::Standalone;
    let mut guard = target
        .streams
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if guard.streams.contains_key(&key) {
        return wire::error_response(
            StatusCode::CONFLICT,
            ErrorCode::INVALID_REQUEST,
            "Conflict: Only one SSE stream is allowed per session",
            Some(&target.id),
        );
    }
    let receiver = insert_stream(&mut guard, key.clone(), None);
    drop(guard);
    let state = stream_state(&target.streams, key, receiver);
    wire::sse_response(Body::from_stream(state), Some(&target.id))
}

fn delete_session(
    sessions: &Mutex<IndexMap<String, Session>>,
    target: &SessionHandle,
) -> Response<Body> {
    let removed = sessions
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .shift_remove(&target.id);
    if let Some(session) = removed {
        let mut streams = session
            .streams
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        streams.streams.clear();
        streams.progress_tokens.clear();
    }
    wire::json_response(StatusCode::OK, Some(&target.id))
}

async fn serve_request(
    shared: &Arc<Mutex<SessionStreams>>,
    id: &str,
    mut service: mpsc::Sender<ClientJsonRpcMessage>,
    call: JsonRpcRequest<ClientRequest>,
) -> Response<Body> {
    let request_id = call.id.clone();
    let progress_token = call.request.get_meta().get_progress_token();
    let key = StreamKey::Request(request_id);
    let receiver = {
        let mut guard = shared.lock().unwrap_or_else(PoisonError::into_inner);
        let receiver = insert_stream(&mut guard, key.clone(), progress_token);
        drop(guard);
        receiver
    };
    if service
        .send(ClientJsonRpcMessage::Request(call))
        .await
        .is_err()
    {
        return terminated(id);
    }
    let state = stream_state(shared, key, receiver);
    wire::sse_response(Body::from_stream(state), Some(id))
}

async fn accept_message(
    service: &mut mpsc::Sender<ClientJsonRpcMessage>,
    message: ClientJsonRpcMessage,
    id: &str,
) -> Response<Body> {
    if service.send(message).await.is_err() {
        return terminated(id);
    }
    wire::json_response(StatusCode::ACCEPTED, Some(id))
}

fn terminated(id: &str) -> Response<Body> {
    wire::error_response(
        StatusCode::NOT_FOUND,
        ErrorCode::INVALID_REQUEST,
        "Not Found: Session has been terminated",
        Some(id),
    )
}
