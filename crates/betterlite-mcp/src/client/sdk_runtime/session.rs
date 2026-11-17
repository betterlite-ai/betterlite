pub mod calls;
pub mod initialize;

use std::collections::BTreeMap;
use std::future::{Future, poll_fn, ready};
use std::io;
use std::pin::{Pin, pin};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::Duration;

use rmcp::RoleClient;
use rmcp::model::{
    ClientCapabilities, ClientRequest, JsonObject, ProgressNotificationParam, ServerResult,
};
use rmcp::service::{Peer, PeerRequestOptions, RunningService, ServiceError, serve_directly};
use rmcp::transport::{Transport, TransportAdapterIdentity};
use tokio::sync::mpsc;

use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::ExitFailure;
use crate::client::sessions::negotiate::ClientCapabilities as CallbackCapabilities;
use crate::client::sessions::runtime::{SessionContext, SessionOptions};
use crate::client::sessions::stream::{MessageHandler, StreamMessage};

use super::convert::{ServerCapabilitiesWire, client_capabilities, request_error};
use super::handler::{SdkClientHandler, progress::ProgressRegistry};
use super::runtime::ProtocolVersionHandle;

pub struct SdkSessionContext<Streams>
where
    Streams: Transport<RoleClient, Error = SdkError> + Send + 'static,
{
    streams: Option<Streams>,
    options: SessionOptions<SdkError>,
    session_handler: MessageHandler<SdkError>,
    protocol_version: Option<ProtocolVersionHandle>,
    progress: Arc<ProgressRegistry>,
    running: Option<RunningService<RoleClient, SdkClientHandler>>,
}

impl<Streams> SdkSessionContext<Streams>
where
    Streams: Transport<RoleClient, Error = SdkError> + Send + 'static,
{
    #[must_use]
    pub fn new(
        streams: Streams,
        options: SessionOptions<SdkError>,
        session_handler: MessageHandler<SdkError>,
        protocol_version: Option<ProtocolVersionHandle>,
    ) -> Self {
        Self {
            streams: Some(streams),
            options,
            session_handler,
            protocol_version,
            progress: Arc::new(ProgressRegistry::default()),
            running: None,
        }
    }
}

impl<Streams> SessionContext for SdkSessionContext<Streams>
where
    Streams: Transport<RoleClient, Error = SdkError> + Send + 'static,
{
    type Session = SdkSession;
    type Error = SdkError;

    fn enter(&mut self) -> impl Future<Output = Result<SdkSession, SdkError>> + Send {
        let Some(streams) = self.streams.take() else {
            return ready(Err(SdkError::AlreadyOpen));
        };
        let handler = SdkClientHandler {
            handlers: self.options.handlers.clone(),
            progress: Arc::clone(&self.progress),
        };
        // The service starts without a handshake because B4744's initialize_session drives it
        let running = serve_directly::<
            RoleClient,
            SdkClientHandler,
            Streams,
            SdkError,
            TransportAdapterIdentity,
        >(handler, streams, None);
        let callbacks = CallbackCapabilities {
            sampling: self.options.handlers.sampling.is_some(),
            elicitation: self.options.handlers.elicitation.is_some(),
        };
        let session = SdkSession {
            peer: running.peer().clone(),
            read_timeout: self.options.read_timeout,
            capabilities: client_capabilities(&callbacks),
            message_handler: self.options.message_handler.clone(),
            session_handler: self.session_handler.clone(),
            protocol_version: self.protocol_version.clone(),
            progress: Arc::clone(&self.progress),
            server_capabilities: None,
            tool_output_schemas: Mutex::new(BTreeMap::new()),
        };
        self.running = Some(running);
        ready(Ok(session))
    }

    async fn exit(mut self) -> Result<(), ExitFailure<SdkError>> {
        let Some(running) = self.running.take() else {
            return Ok(());
        };
        match running.cancel().await {
            Ok(_reason) => Ok(()),
            Err(cause) => Err(ExitFailure::Failure(Arc::new(SdkError::Io(Arc::new(
                io::Error::other(cause),
            ))))),
        }
    }
}

pub struct SdkSession {
    pub peer: Peer<RoleClient>,
    pub read_timeout: Duration,
    pub capabilities: ClientCapabilities,
    pub message_handler: MessageHandler<SdkError>,
    pub session_handler: MessageHandler<SdkError>,
    // the streamable transport's version setter, absent when the session runs on stdio or legacy SSE
    pub protocol_version: Option<ProtocolVersionHandle>,
    pub progress: Arc<ProgressRegistry>,
    // the initialize result's capabilities on the wire, for betterlite-core's typed decode
    pub server_capabilities: Option<ServerCapabilitiesWire>,
    // every output schema the session has seen, keyed by tool name, as the SDK caches them
    pub tool_output_schemas: Mutex<BTreeMap<String, Option<JsonObject>>>,
}

// A progress sink the callers that take none use, so one exchange path serves every request
struct Silent;

trait ProgressSink: Sync {
    fn send(&self, param: ProgressNotificationParam) -> impl Future<Output = ()> + Send;
}

impl ProgressSink for Silent {
    fn send(&self, _param: ProgressNotificationParam) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

// One request's exchange, with the progress notifications a call forwards while it waits for the answer
async fn exchange_with_progress<Sink>(
    session: &SdkSession,
    request: ClientRequest,
    sink: &Sink,
) -> Result<ServerResult, SdkError>
where
    Sink: ProgressSink,
{
    let method = request.method().to_owned();
    let options = PeerRequestOptions::with_timeout(session.read_timeout);
    let handle = session
        .peer
        .send_request_with_option(request, options)
        .await
        .map_err(|error| request_failure(session, &method, error))?;
    let token = handle.progress_token.clone();
    let (mut progress, _subscription) = session.progress.subscribe(&token);
    let response = forward_progress(&mut progress, pin!(handle.await_response()), sink).await;
    response.map_err(|error| request_failure(session, &method, error))
}

// The answer is awaited while the call's progress updates go to the sink, one per poll
async fn forward_progress<Sink, Response>(
    progress: &mut mpsc::Receiver<ProgressNotificationParam>,
    mut pending: Pin<&mut Response>,
    sink: &Sink,
) -> Result<ServerResult, ServiceError>
where
    Sink: ProgressSink,
    Response: Future<Output = Result<ServerResult, ServiceError>>,
{
    loop {
        let mut delivered = None;
        let finished = poll_fn(|context| {
            if let Poll::Ready(result) = pending.as_mut().poll(context) {
                return Poll::Ready(Some(result));
            }
            match progress.poll_recv(context) {
                Poll::Ready(Some(param)) => {
                    delivered = Some(param);
                    Poll::Ready(None)
                }
                Poll::Ready(None) | Poll::Pending => Poll::Pending,
            }
        })
        .await;
        let Some(result) = finished else {
            if let Some(param) = delivered {
                sink.send(param).await;
            }
            continue;
        };
        // Updates queued before the answer resolved still belong to the call, so they go out before it returns
        while let Ok(param) = progress.try_recv() {
            sink.send(param).await;
        }
        return result;
    }
}

fn request_failure(session: &SdkSession, method: &str, error: ServiceError) -> SdkError {
    let recorded = session
        .session_handler
        .recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(failure) = recorded {
        // B4744 watches its own handler, so a stream failure the transport saw is replayed there before the call fails
        let _closed = session
            .message_handler
            .handle(StreamMessage::Failure((*failure).clone()));
    }
    request_error(method, error)
}
