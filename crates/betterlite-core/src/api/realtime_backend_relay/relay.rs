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
use super::transformed::handle_provider_config_message;

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
    // ast-grep-ignore: no-long-names
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
    // ast-grep-ignore: no-long-names
    async fn backend_to_client_send_messages(
        &mut self,
        source: &mut (dyn RealtimeBackendSource + '_),
        cancellation: &CancellationToken,
    ) -> Result<BackendClose, RealtimeRelayError<Config::Error, CallbackError>> {
        let exit = loop {
            match relay_once(self, source, cancellation).await {
                Ok(()) => {}
                Err(RealtimeRelayError::Cancelled) => break RelayExit::Cancelled,
                Err(RealtimeRelayError::Receive(RelayReceiveError::Closed(closed))) => {
                    break RelayExit::Closed(closed);
                }
                Err(error) => break RelayExit::Failed(error),
            }
        };
        finish(self, exit)
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
    let Some(received) = cancellation
        .run_until_cancelled(source.recv(Some(false)))
        .await
    else {
        return Err(RealtimeRelayError::Cancelled);
    };
    let message = match received {
        Ok(message) => message,
        Err(RelayReceiveError::UnsupportedDecode) => source
            .recv(None)
            .await
            .map_err(RealtimeRelayError::Receive)?,
        Err(error) => return Err(RealtimeRelayError::Receive(error)),
    };
    relay.state.backend_sent_frames.0 = true;
    let text = match message {
        RealtimeMessage::Text(text) => text,
        RealtimeMessage::Binary(bytes) => {
            let Ok(text) = String::from_utf8(bytes) else {
                tracing::warn!("Received non-UTF-8 binary frame from backend, skipping.");
                return Ok(());
            };
            text
        }
    };
    if let Some(provider) = relay.provider {
        if let Err(error) = handle_provider_config_message(relay, provider, &text).await {
            tracing::error!("Error processing backend message, skipping: {error}");
        }
        return Ok(());
    }
    relay_raw_frame(relay, &text).await
}

// LiteLLM logs every close at error level, flushes unbilled usage, then classifies a pre-frame non-1000 close as a refusal
fn finish<Config, CallbackError>(
    relay: &mut RealtimeBackendRelay<'_, Config, CallbackError>,
    exit: RelayExit<Config::Error, CallbackError>,
) -> Result<BackendClose, RealtimeRelayError<Config::Error, CallbackError>>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    match exit {
        RelayExit::Closed(closed) => {
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
        RelayExit::Cancelled => {
            relay
                .state
                .flush_unbilled_usage()
                .map_err(RealtimeRelayError::State)?;
            let _logged = relay.state.log_messages();
            Err(RealtimeRelayError::Cancelled)
        }
        RelayExit::Failed(error) => {
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
    }
}
