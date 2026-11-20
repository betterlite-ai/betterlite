use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use tokio_util::sync::CancellationToken;

use crate::api::realtime_contracts::close::BackendClose;
use crate::api::realtime_contracts::close::ConnectionClosed;
use crate::api::realtime_contracts::close::backend_close_from;
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::transport::RealtimeMessage;

use super::RealtimeBackendRelay;
use super::error::RealtimeRelayError;
use super::raw::relay_raw_frame;
use super::source::BackendRefusal;
use super::source::RealtimeBackendSource;
use super::source::RelayReceiveError;
use super::transformed::ProviderConfigRelay as _;

pub const FAILURE_LOGGED_KEY: &str = "realtime_session_failure_logged";

const PROXY_RELAY_FAILURE_REASON: &str = "proxy failed while relaying the upstream websocket";

// How LiteLLM's NoReturn relay ended: a backend close, a cancelled caller, or a failure the proxy answers with 1011
pub enum RelayExit<ConfigError, CallbackError>
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    Closed(ConnectionClosed),
    Cancelled,
    Failed(RealtimeRelayError<ConfigError, CallbackError>),
}

#[async_trait]
pub trait BackendToClientRelay<Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    async fn relay_backend_messages(
        &mut self,
        source: &mut (dyn RealtimeBackendSource + '_),
        cancellation: &CancellationToken,
    ) -> RelayExit<Config::Error, CallbackError>;

    async fn backend_to_client_send_messages(
        &mut self,
        source: &mut (dyn RealtimeBackendSource + '_),
        cancellation: &CancellationToken,
    ) -> Result<BackendClose, RealtimeRelayError<Config::Error, CallbackError>>;

    fn backend_refused_session(&self, close: &BackendClose) -> bool;

    fn log_backend_refusal(&mut self, error: &ConnectionClosed);
}

#[async_trait]
impl<Config, CallbackError> BackendToClientRelay<Config, CallbackError>
    for RealtimeBackendRelay<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    async fn relay_backend_messages(
        &mut self,
        source: &mut (dyn RealtimeBackendSource + '_),
        cancellation: &CancellationToken,
    ) -> RelayExit<Config::Error, CallbackError> {
        loop {
            match relay_once(self, source, cancellation).await {
                Ok(()) => {}
                Err(RealtimeRelayError::Cancelled) => return RelayExit::Cancelled,
                Err(RealtimeRelayError::Receive(RelayReceiveError::Closed(closed))) => {
                    return RelayExit::Closed(closed);
                }
                Err(
                    error @ (RealtimeRelayError::Receive(RelayReceiveError::UnsupportedDecode)
                    | RealtimeRelayError::Receive(RelayReceiveError::Failed(_))
                    | RealtimeRelayError::Transform(_)
                    | RealtimeRelayError::State(_)
                    | RealtimeRelayError::Predicate(_)
                    | RealtimeRelayError::Send(_)
                    | RealtimeRelayError::Guardrails(_)
                    | RealtimeRelayError::SetupBuffer(_)
                    | RealtimeRelayError::ClientMissing
                    | RealtimeRelayError::Client(_)
                    | RealtimeRelayError::Encode(_)
                    | RealtimeRelayError::Beta(_)),
                ) => return RelayExit::Failed(error),
            }
        }
    }

    async fn backend_to_client_send_messages(
        &mut self,
        source: &mut (dyn RealtimeBackendSource + '_),
        cancellation: &CancellationToken,
    ) -> Result<BackendClose, RealtimeRelayError<Config::Error, CallbackError>> {
        match self.relay_backend_messages(source, cancellation).await {
            RelayExit::Closed(closed) => finish_closed(self, closed),
            RelayExit::Cancelled => finish_cancelled(self),
            RelayExit::Failed(error) => finish_failed(self, error),
        }
    }

    fn backend_refused_session(&self, close: &BackendClose) -> bool {
        if close.code == 1000 {
            return false;
        }
        !self.state.backend_sent_frames.0
    }

    fn log_backend_refusal(&mut self, error: &ConnectionClosed) {
        let Some(logging) = self.state.logging.as_deref() else {
            return;
        };
        let close = backend_close_from(error);
        let refusal = BackendRefusal {
            code: close.code,
            message: error.to_string(),
        };
        let work = self
            .failure_dispatch
            .dispatch_failure_handlers(&refusal, true);
        if let Some(queue) = self.state.logging_worker.as_deref() {
            queue.enqueue(work);
        }
        logging.record_model_call_detail(FAILURE_LOGGED_KEY, Json::Bool(true));
    }
}

// One relay step: wait for a frame, mark that the backend sent one, then run the transformed or the raw branch
async fn relay_once<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    source: &mut (dyn RealtimeBackendSource + '_),
    cancellation: &CancellationToken,
) -> Result<(), RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    let received = tokio::select! {
        biased;
        () = cancellation.cancelled() => return Err(RealtimeRelayError::Cancelled),
        received = source.recv(Some(false)) => received,
    };
    let message = match received {
        Ok(message) => message,
        Err(RelayReceiveError::UnsupportedDecode) => source
            .recv(None)
            .await
            .map_err(RealtimeRelayError::Receive)?,
        Err(RelayReceiveError::Closed(error)) => {
            return Err(RealtimeRelayError::Receive(RelayReceiveError::Closed(
                error,
            )));
        }
        Err(RelayReceiveError::Failed(error)) => {
            return Err(RealtimeRelayError::Receive(RelayReceiveError::Failed(
                error,
            )));
        }
    };
    relay.state.backend_sent_frames.0 = true;
    let text = match message {
        RealtimeMessage::Text(text) => text,
        RealtimeMessage::Binary(bytes) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                tracing::warn!("Received non-UTF-8 binary frame from backend, skipping.");
                return Ok(());
            }
        },
    };
    if let Some(provider) = relay.provider {
        if let Err(error) = relay.handle_provider_config_message(provider, &text).await {
            tracing::error!("Error processing backend message, skipping: {error}");
        }
        return Ok(());
    }
    relay_raw_frame(relay, &text).await
}

// LiteLLM logs every close at error level, flushes unbilled usage, then classifies a pre-frame non-1000 close as a refusal
fn finish_closed<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    closed: ConnectionClosed,
) -> Result<BackendClose, RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    tracing::error!("Connection closed in backend to client send messages - {closed}");
    let close = backend_close_from(&closed);
    relay
        .state
        .flush_unbilled_usage()
        .map_err(RealtimeRelayError::State)?;
    if relay.backend_refused_session(&close) {
        relay.log_backend_refusal(&closed);
    } else {
        let _logged = relay.state.log_messages();
    }
    Ok(close)
}

fn finish_cancelled<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
) -> Result<BackendClose, RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    relay
        .state
        .flush_unbilled_usage()
        .map_err(RealtimeRelayError::State)?;
    let _logged = relay.state.log_messages();
    Err(RealtimeRelayError::Cancelled)
}

fn finish_failed<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    error: RealtimeRelayError<Config::Error, CallbackError>,
) -> Result<BackendClose, RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    tracing::error!("Error in backend to client send messages: {error}");
    relay
        .state
        .flush_unbilled_usage()
        .map_err(RealtimeRelayError::State)?;
    let _logged = relay.state.log_messages();
    Ok(BackendClose {
        code: 1011,
        reason: String::from(PROXY_RELAY_FAILURE_REASON),
    })
}
