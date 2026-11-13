use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode};
use futures_channel::mpsc;
use futures_util::SinkExt as _;
use indexmap::IndexMap;
use rmcp::ServerHandler;
use rmcp::model::{
    ClientJsonRpcMessage, ClientNotification, ClientRequest, ErrorCode, GetMeta as _,
    JsonRpcRequest, RequestId,
};

use super::session::{
    Session, SessionMap, SessionStreams, StreamKey, cancel_request, insert_stream, start_session,
    stream_state,
};
use super::wire;

struct SessionHandle {
    id: String,
    streams: Arc<Mutex<SessionStreams>>,
    service: mpsc::Sender<ClientJsonRpcMessage>,
}

pub(super) async fn handle<H>(
    sessions: &SessionMap,
    request: Request<Body>,
    handler: H,
) -> Response<Body>
where
    H: ServerHandler,
{
    let id = match session_header(&request) {
        SessionHeader::Absent => {
            let id = uuid::Uuid::new_v4().simple().to_string();
            return new_session_response(sessions, &id, request, handler).await;
        }
        SessionHeader::Invalid => return unknown_session(),
        SessionHeader::Id(id) => id,
    };
    let Some(target) = lookup(sessions, &id) else {
        return unknown_session();
    };
    match *request.method() {
        Method::POST => existing_post(target, request).await,
        Method::GET => existing_get(&target, &request),
        Method::DELETE => delete_session(sessions, &target),
        _ => wire::method_not_allowed(Some(&target.id)),
    }
}

async fn new_session_response<H>(
    sessions: &SessionMap,
    id: &str,
    request: Request<Body>,
    handler: H,
) -> Response<Body>
where
    H: ServerHandler,
{
    match *request.method() {
        Method::POST => new_session_post(sessions, id, request, handler).await,
        Method::GET => new_session_get(id, &request),
        Method::DELETE => missing_session_id(id),
        _ => wire::method_not_allowed(Some(id)),
    }
}

enum SessionHeader {
    Absent,
    Invalid,
    Id(String),
}

fn session_header(request: &Request<Body>) -> SessionHeader {
    let Some(value) = request.headers().get(wire::MCP_SESSION_ID_HEADER) else {
        return SessionHeader::Absent;
    };
    value.to_str().map_or(SessionHeader::Invalid, |id| {
        SessionHeader::Id(id.to_owned())
    })
}

fn lookup(sessions: &Mutex<IndexMap<String, Session>>, id: &str) -> Option<SessionHandle> {
    let map = sessions.lock().unwrap_or_else(PoisonError::into_inner);
    let entry = map.get(id)?;
    let service = entry.service.clone();
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

async fn new_session_post<H>(
    sessions: &SessionMap,
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
    let mut session = start_session(id, &Arc::downgrade(sessions), handler);
    let response = serve_request(&session.streams, id, &mut session.service, call).await;
    keep_if_live(sessions, id.to_owned(), session);
    response
}

fn keep_if_live(sessions: &Mutex<IndexMap<String, Session>>, id: String, session: Session) {
    let mut map = sessions.lock().unwrap_or_else(PoisonError::into_inner);
    if session.ended.load(Ordering::Acquire) {
        drop(map);
        return;
    }
    map.insert(id, session);
}

async fn existing_post(target: SessionHandle, request: Request<Body>) -> Response<Body> {
    let message = match wire::decode_post(request, Some(&target.id)).await {
        Ok(message) => message,
        Err(response) => return *response,
    };
    let cancelled = cancelled_request(&message);
    let mut service = target.service;
    let response = match message {
        ClientJsonRpcMessage::Request(call) => {
            serve_request(&target.streams, &target.id, &mut service, call).await
        }
        other @ (ClientJsonRpcMessage::Response(_)
        | ClientJsonRpcMessage::Notification(_)
        | ClientJsonRpcMessage::Error(_)) => accept_message(&mut service, other, &target.id).await,
    };
    if let Some(request_id) = cancelled {
        cancel_request(&target.streams, &request_id).await;
    }
    response
}

fn cancelled_request(message: &ClientJsonRpcMessage) -> Option<RequestId> {
    let ClientJsonRpcMessage::Notification(notification) = message else {
        return None;
    };
    let ClientNotification::CancelledNotification(cancelled) = &notification.notification else {
        return None;
    };
    cancelled.params.request_id.clone()
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
        streams.close_all();
    }
    wire::json_body(StatusCode::OK, Body::empty(), Some(&target.id))
}

async fn serve_request(
    shared: &Arc<Mutex<SessionStreams>>,
    id: &str,
    service: &mut mpsc::Sender<ClientJsonRpcMessage>,
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
    let state = stream_state(shared, key, receiver);
    if service
        .send(ClientJsonRpcMessage::Request(call))
        .await
        .is_err()
    {
        drop(state);
        return terminated(id);
    }
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
    wire::json_body(StatusCode::ACCEPTED, Body::empty(), Some(id))
}

fn terminated(id: &str) -> Response<Body> {
    wire::error_response(
        StatusCode::NOT_FOUND,
        ErrorCode::INVALID_REQUEST,
        "Not Found: Session has been terminated",
        Some(id),
    )
}

mod tests;
