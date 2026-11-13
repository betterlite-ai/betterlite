use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::task::{Context, Poll};

use axum::body::Bytes;
use futures_channel::mpsc;
use futures_util::SinkExt as _;
use futures_util::StreamExt as _;
use futures_util::stream::Stream;
use indexmap::IndexMap;
use rmcp::model::{
    ClientJsonRpcMessage, ErrorCode, ErrorData, GetExtensions as _, JsonRpcMessage, ProgressToken,
    RequestId, ServerNotification,
};
use rmcp::service::{OriginatingRequestId, TxJsonRpcMessage};
use rmcp::{RoleServer, ServerHandler, serve_server};
use tokio::task::JoinSet;
use tokio::time::{Interval, interval};

use super::wire;

pub(super) const SERVICE_CHANNEL_CAPACITY: usize = 32;
pub(super) const REQUEST_STREAM_CAPACITY: usize = 16;
// LSP's RequestCancelled, the code the SDK writes for a request that settled unanswered
const REQUEST_CANCELLED: i32 = -32800;

pub(super) type SessionMap = Arc<Mutex<IndexMap<String, Session>>>;

pub(super) struct Session {
    pub(super) streams: Arc<Mutex<SessionStreams>>,
    pub(super) service: mpsc::Sender<ClientJsonRpcMessage>,
    pub(super) ended: Arc<AtomicBool>,
    // holding the tasks here stops them with the session, so only the drop matters
    _tasks: JoinSet<()>,
}

pub(super) fn start_session<H>(
    id: &str,
    sessions: &Weak<Mutex<IndexMap<String, Session>>>,
    handler: H,
) -> Session
where
    H: ServerHandler,
{
    let (to_service, to_service_rx) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
    let (from_service, from_service_rx) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
    let streams = Arc::new(Mutex::new(SessionStreams::default()));
    let ended = Arc::new(AtomicBool::new(false));
    let mut tasks = JoinSet::new();
    let service_streams = Arc::clone(&streams);
    let service_ended = Arc::clone(&ended);
    let service_sessions = Weak::clone(sessions);
    let service_id = id.to_owned();
    tasks.spawn(async move {
        match serve_server(handler, (from_service, to_service_rx)).await {
            Ok(running) => drop(running.waiting().await),
            Err(error) => tracing::warn!(%error, "an MCP streamable HTTP session failed"),
        }
        discard_session(
            &service_sessions,
            &service_id,
            &service_streams,
            &service_ended,
        );
    });
    let router_streams = Arc::clone(&streams);
    let router_ended = Arc::clone(&ended);
    let router_sessions = Weak::clone(sessions);
    let router_id = id.to_owned();
    tasks.spawn(async move {
        route_session_messages(&router_streams, from_service_rx).await;
        discard_session(&router_sessions, &router_id, &router_streams, &router_ended);
    });
    Session {
        streams,
        service: to_service,
        ended,
        _tasks: tasks,
    }
}

fn discard_session(
    sessions: &Weak<Mutex<IndexMap<String, Session>>>,
    id: &str,
    streams: &Arc<Mutex<SessionStreams>>,
    ended: &AtomicBool,
) {
    streams
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .close_all();
    let Some(live) = sessions.upgrade() else {
        ended.store(true, Ordering::Release);
        return;
    };
    let mut map = live.lock().unwrap_or_else(PoisonError::into_inner);
    ended.store(true, Ordering::Release);
    map.shift_remove(id);
}

#[derive(Default)]
pub(super) struct SessionStreams {
    pub(super) streams: IndexMap<StreamKey, mpsc::Sender<Bytes>>,
    pub(super) progress_tokens: HashMap<ProgressToken, RequestId>,
}

impl SessionStreams {
    pub(super) fn close_all(&mut self) {
        self.streams.clear();
        self.progress_tokens.clear();
    }

    fn close(&mut self, key: &StreamKey) {
        self.streams.shift_remove(key);
        if let StreamKey::Request(request_id) = key {
            self.progress_tokens
                .retain(|_, routed| routed != request_id);
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum StreamKey {
    Request(RequestId),
    Standalone,
}

async fn route_session_messages(
    shared: &Mutex<SessionStreams>,
    mut outgoing: mpsc::Receiver<TxJsonRpcMessage<RoleServer>>,
) {
    while let Some(message) = outgoing.next().await {
        let key = {
            let streams = shared.lock().unwrap_or_else(PoisonError::into_inner);
            routed_request(&message, &streams.progress_tokens)
                .map_or(StreamKey::Standalone, StreamKey::Request)
        };
        let terminal = terminal_request(&message).is_some();
        let sender = shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .streams
            .get(&key)
            .cloned();
        let Some(mut sink) = sender else {
            continue;
        };
        if sink.send(wire::sse_event(&message)).await.is_err() {
            shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .close(&key);
            continue;
        }
        if terminal {
            shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .close(&key);
        }
    }
}

pub(super) fn routed_request(
    message: &TxJsonRpcMessage<RoleServer>,
    progress_tokens: &HashMap<ProgressToken, RequestId>,
) -> Option<RequestId> {
    match message {
        JsonRpcMessage::Response(response) => Some(response.id.clone()),
        JsonRpcMessage::Error(error) => error.id.clone(),
        JsonRpcMessage::Request(request) => request
            .request
            .extensions()
            .get::<OriginatingRequestId>()
            .map(|originating| originating.0.clone()),
        JsonRpcMessage::Notification(notification) => {
            let ServerNotification::ProgressNotification(progress) = &notification.notification
            else {
                return None;
            };
            progress_tokens
                .get(&progress.params.progress_token)
                .cloned()
        }
    }
}

pub(super) const fn terminal_request(message: &TxJsonRpcMessage<RoleServer>) -> Option<&RequestId> {
    match message {
        JsonRpcMessage::Response(response) => Some(&response.id),
        JsonRpcMessage::Error(error) => error.id.as_ref(),
        JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => None,
    }
}

// The SDK answers a request that settled unanswered with -32800 and ends its stream
pub(super) async fn cancel_request(shared: &Arc<Mutex<SessionStreams>>, request_id: &RequestId) {
    let key = StreamKey::Request(request_id.clone());
    let sender = shared
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .streams
        .get(&key)
        .cloned();
    let Some(mut sink) = sender else {
        return;
    };
    let error = JsonRpcMessage::error(
        ErrorData::new(ErrorCode(REQUEST_CANCELLED), "Request cancelled", None),
        Some(request_id.clone()),
    );
    let _sent = sink.send(wire::sse_event(&error)).await;
    shared
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .close(&key);
}

pub(super) struct SseState {
    events: mpsc::Receiver<Bytes>,
    ping: Interval,
    // dropping the body releases the stream entry, so only the drop matters
    _release: StreamRelease,
}

pub(super) struct StreamRelease {
    shared: Arc<Mutex<SessionStreams>>,
    key: StreamKey,
}

impl Stream for SseState {
    type Item = Result<Bytes, Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Poll::Ready(event) = Pin::new(&mut this.events).poll_next(cx) {
            return Poll::Ready(event.map(Ok));
        }
        match this.ping.poll_tick(cx) {
            Poll::Ready(_) => Poll::Ready(Some(Ok(wire::ping()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for StreamRelease {
    fn drop(&mut self) {
        self.shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .close(&self.key);
    }
}

pub(super) fn ping_interval() -> Interval {
    let mut ping = interval(wire::PING_INTERVAL);
    ping.reset();
    ping
}

pub(super) fn insert_stream(
    streams: &mut SessionStreams,
    key: StreamKey,
    progress_token: Option<ProgressToken>,
) -> mpsc::Receiver<Bytes> {
    let (sender, receiver) = mpsc::channel(REQUEST_STREAM_CAPACITY);
    if let (StreamKey::Request(request_id), Some(token)) = (&key, progress_token) {
        streams.progress_tokens.insert(token, request_id.clone());
    }
    streams.streams.insert(key, sender);
    receiver
}

pub(super) fn stream_state(
    shared: &Arc<Mutex<SessionStreams>>,
    key: StreamKey,
    events: mpsc::Receiver<Bytes>,
) -> SseState {
    SseState {
        events,
        ping: ping_interval(),
        _release: StreamRelease {
            shared: Arc::clone(shared),
            key,
        },
    }
}
