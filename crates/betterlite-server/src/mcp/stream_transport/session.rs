use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use axum::body::Bytes;
use futures_channel::mpsc;
use futures_util::SinkExt as _;
use futures_util::StreamExt as _;
use futures_util::stream::Stream;
use indexmap::IndexMap;
use rmcp::model::{
    ClientJsonRpcMessage, GetExtensions as _, JsonRpcMessage, ProgressToken, RequestId,
    ServerNotification,
};
use rmcp::service::{OriginatingRequestId, TxJsonRpcMessage};
use rmcp::{RoleServer, ServerHandler, serve_server};
use tokio::task::JoinSet;
use tokio::time::{Interval, interval};

use super::wire;

pub(super) const SERVICE_CHANNEL_CAPACITY: usize = 32;
pub(super) const REQUEST_STREAM_CAPACITY: usize = 16;

pub(super) struct Session {
    pub(super) streams: Arc<Mutex<SessionStreams>>,
    pub(super) service: Option<mpsc::Sender<ClientJsonRpcMessage>>,
    pub(super) tasks: JoinSet<()>,
}

impl Session {
    pub(super) fn new() -> Self {
        Self {
            streams: Arc::new(Mutex::new(SessionStreams::default())),
            service: None,
            tasks: JoinSet::new(),
        }
    }

    pub(super) fn start<H>(&mut self, handler: H)
    where
        H: ServerHandler,
    {
        let (to_service, to_service_rx) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
        let (from_service, from_service_rx) = mpsc::channel(SERVICE_CHANNEL_CAPACITY);
        self.tasks.spawn(async move {
            match serve_server(handler, (from_service, to_service_rx)).await {
                Ok(running) => drop(running.waiting().await),
                Err(error) => tracing::warn!(%error, "an MCP streamable HTTP session failed"),
            }
        });
        let streams = Arc::clone(&self.streams);
        self.tasks
            .spawn(route_session_messages(streams, from_service_rx));
        self.service = Some(to_service);
    }
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
    shared: Arc<Mutex<SessionStreams>>,
    mut outgoing: mpsc::Receiver<TxJsonRpcMessage<RoleServer>>,
) {
    while let Some(message) = outgoing.next().await {
        let key = stream_key(&message, &shared);
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

fn stream_key(message: &TxJsonRpcMessage<RoleServer>, shared: &Mutex<SessionStreams>) -> StreamKey {
    match message {
        JsonRpcMessage::Response(response) => StreamKey::Request(response.id.clone()),
        JsonRpcMessage::Error(error) => error
            .id
            .as_ref()
            .map_or(StreamKey::Standalone, |id| StreamKey::Request(id.clone())),
        JsonRpcMessage::Request(request) => request
            .request
            .extensions()
            .get::<OriginatingRequestId>()
            .map_or(StreamKey::Standalone, |originating| {
                StreamKey::Request(originating.0.clone())
            }),
        JsonRpcMessage::Notification(notification) => {
            let ServerNotification::ProgressNotification(progress) = &notification.notification
            else {
                return StreamKey::Standalone;
            };
            shared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .progress_tokens
                .get(&progress.params.progress_token)
                .map_or(StreamKey::Standalone, |id| StreamKey::Request(id.clone()))
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

pub(super) struct SseState {
    events: mpsc::Receiver<Bytes>,
    ping: Interval,
    release: Option<StreamRelease>,
}

pub(super) struct StreamRelease {
    shared: Arc<Mutex<SessionStreams>>,
    key: StreamKey,
}

impl SseState {
    pub(super) fn new(events: mpsc::Receiver<Bytes>) -> Self {
        Self {
            events,
            ping: ping_interval(),
            release: None,
        }
    }
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
    receiver: mpsc::Receiver<Bytes>,
) -> SseState {
    let mut state = SseState::new(receiver);
    state.release = Some(StreamRelease {
        shared: Arc::clone(shared),
        key,
    });
    state
}
