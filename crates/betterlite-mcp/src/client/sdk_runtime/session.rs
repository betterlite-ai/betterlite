pub mod calls;
mod initialize;

use std::future::Future;
use std::pin::pin;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use rmcp::RoleClient;
use rmcp::model::{ClientCapabilities, ClientRequest, ProgressNotificationParam, ServerResult};
use rmcp::service::{Peer, PeerRequestOptions, RunningService, ServiceError, serve_directly};
use rmcp::transport::{Transport, TransportAdapterIdentity};

use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::ExitFailure;
use crate::client::sessions::runtime::{SessionContext, SessionOptions};
use crate::client::sessions::stream::{MessageHandler, StreamMessage};

use super::convert::{ServerCapabilitiesWire, request_error};
use super::handler::{ProgressRegistry, SdkClientHandler};

pub struct SdkSessionContext<Streams>
where
    Streams: Transport<RoleClient, Error = SdkError> + Send + 'static,
{
    streams: Option<Streams>,
    options: SessionOptions<SdkError>,
    session_handler: MessageHandler<SdkError>,
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
    ) -> Self {
        Self {
            streams: Some(streams),
            options,
            session_handler,
            progress: Arc::new(ProgressRegistry::default()),
            running: None,
        }
    }

    // A runtime shares its stream handler across sessions, so the previous session's failure must not fail this one
    fn forget_stream_failure(&self) {
        let mut recorded = self
            .session_handler
            .recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *recorded = None;
    }
}

impl<Streams> SessionContext for SdkSessionContext<Streams>
where
    Streams: Transport<RoleClient, Error = SdkError> + Send + 'static,
{
    type Session = SdkSession;
    type Error = SdkError;

    async fn enter(&mut self) -> Result<SdkSession, SdkError> {
        let Some(streams) = self.streams.take() else {
            return Err(SdkError::AlreadyOpen);
        };
        self.forget_stream_failure();
        let handler =
            SdkClientHandler::new(self.options.handlers.clone(), Arc::clone(&self.progress));
        // The service starts without a handshake because B4744's initialize_session drives it
        let running = serve_directly::<
            RoleClient,
            SdkClientHandler,
            Streams,
            SdkError,
            TransportAdapterIdentity,
        >(handler, streams, None);
        let session = SdkSession {
            peer: running.peer().clone(),
            read_timeout: self.options.read_timeout,
            capabilities: self.options.handlers.capabilities(),
            message_handler: self.options.message_handler.clone(),
            session_handler: self.session_handler.clone(),
            progress: Arc::clone(&self.progress),
            server_capabilities: None,
        };
        self.running = Some(running);
        Ok(session)
    }

    async fn exit(mut self) -> Result<(), ExitFailure<SdkError>> {
        let Some(running) = self.running.take() else {
            return Ok(());
        };
        match running.cancel().await {
            Ok(_reason) => Ok(()),
            Err(cause) => Err(ExitFailure::Failure(Arc::new(SdkError::Io(Arc::new(
                std::io::Error::other(cause),
            ))))),
        }
    }
}

pub struct SdkSession {
    peer: Peer<RoleClient>,
    read_timeout: Duration,
    capabilities: ClientCapabilities,
    message_handler: MessageHandler<SdkError>,
    session_handler: MessageHandler<SdkError>,
    progress: Arc<ProgressRegistry>,
    // the initialize result's capabilities on the wire, for betterlite-core's typed decode
    pub server_capabilities: Option<ServerCapabilitiesWire>,
}

// A progress sink the callers that take none use, so one exchange path serves every request
struct Silent;

trait ProgressSink {
    fn send(&self, param: ProgressNotificationParam) -> impl Future<Output = ()> + Send;
}

impl ProgressSink for Silent {
    async fn send(&self, _param: ProgressNotificationParam) {}
}

impl SdkSession {
    async fn exchange(&self, request: ClientRequest) -> Result<ServerResult, SdkError> {
        self.exchange_with_progress(request, &Silent).await
    }

    async fn exchange_with_progress<Sink>(
        &self,
        request: ClientRequest,
        sink: &Sink,
    ) -> Result<ServerResult, SdkError>
    where
        Sink: ProgressSink,
    {
        let method = request.method().to_owned();
        let options = PeerRequestOptions::with_timeout(self.read_timeout);
        let handle = self
            .peer
            .send_request_with_option(request, options)
            .await
            .map_err(|error| self.request_failure(&method, error))?;
        let token = handle.progress_token.clone();
        let mut progress = self.progress.subscribe(&token);
        let response = {
            let mut pending = pin!(handle.await_response());
            let mut closed = false;
            loop {
                tokio::select! {
                    result = &mut pending => break result,
                    update = progress.recv(), if !closed => match update {
                        Some(param) => sink.send(param).await,
                        None => closed = true,
                    },
                }
            }
        };
        self.progress.unsubscribe(&token);
        response.map_err(|error| self.request_failure(&method, error))
    }

    fn request_failure(&self, method: &str, error: ServiceError) -> SdkError {
        let recorded = self
            .session_handler
            .recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(failure) = recorded {
            // B4744 watches its own handler, so a stream failure the transport saw is replayed there before the call fails
            let _closed = self
                .message_handler
                .handle(StreamMessage::Failure((*failure).clone()));
        }
        request_error(method, error)
    }
}
