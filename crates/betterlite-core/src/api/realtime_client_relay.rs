mod ack;
mod close_client;
pub mod error;
mod rewrite;
mod session;

mod tests;

use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use betterlite_redact::secret::Redactor;
use futures_util::future::{Either, select};
use tokio_util::sync::CancellationToken;

use crate::api::realtime_contracts::close::{BackendClose, ClientLoopExit};
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
use crate::api::realtime_contracts::transport::ClientWebSocket;
use crate::api::realtime_guardrail_policy::RealtimeGuardrailCallback;
use crate::api::realtime_guardrails::callback::RealtimeGuardrail;
use crate::api::realtime_sender::RealtimeSender;
use crate::api::realtime_session_state::RealtimeStreaming;

// LiteLLM's client-to-backend relay: B1006's connection state, B1004's provider config, B2455/B1007's guardrails and B187's redactor
pub struct RealtimeClientRelay<'relay, Config, CallbackError = Infallible>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    pub state: &'relay mut RealtimeStreaming,
    pub provider: Option<&'relay Config>,
    pub guardrails: &'relay [&'relay dyn RealtimeGuardrail<Error = CallbackError>],
    pub callbacks: Vec<&'relay dyn RealtimeGuardrailCallback<Error = CallbackError>>,
    pub redactor: Option<&'relay Redactor>,
}

impl<Config, CallbackError> RealtimeClientRelay<'_, Config, CallbackError>
where
    Config: BaseRealtimeConfig,
    CallbackError: Error + Send + 'static,
{
    // LiteLLM's _receive_client_message: a closed or broken client socket is a disconnect, not a loop failure
    pub async fn receive_client_message(&mut self) -> Option<String> {
        let Some(socket) = self.state.client.as_deref_mut() else {
            tracing::debug!("Client disconnected: the realtime client websocket is not connected");
            return None;
        };
        match socket.receive_text().await {
            Ok(message) => Some(message),
            Err(error) => {
                tracing::debug!("Client disconnected: {error}");
                None
            }
        }
    }

    pub async fn client_ack_messages(&mut self) -> ClientLoopExit {
        loop {
            let Some(message) = self.receive_client_message().await else {
                return ClientLoopExit::ClientDisconnected;
            };
            match ack::handle(self, &message).await {
                Ok(()) => {}
                Err(error) if error.is_backend_closed() => {
                    tracing::debug!("Backend closed while forwarding a client message: {error}");
                    return ClientLoopExit::BackendClosed;
                }
                Err(error) => {
                    tracing::debug!("Error in client ack messages: {error}");
                    return ClientLoopExit::ClientDisconnected;
                }
            }
        }
    }

    // LiteLLM's _close_client: one failed attempt is logged and swallowed, so a failed notice also skips the close
    pub async fn close_client(&mut self, close: &BackendClose) {
        let message = self.redact(&close.message());
        let reason = self.redact(&close.reason);
        close_client::relay(&mut self.state.client, close.code, &message, &reason).await;
    }

    // LiteLLM's bidirectional_forward: wait for the first direction, close on either completion, then cancel and join both
    pub async fn bidirectional_forward<ClientDirection, BackendDirection, BackendError>(
        &mut self,
        client_direction: ClientDirection,
        backend_direction: BackendDirection,
        cancellation: &CancellationToken,
    ) -> Result<(), BackendError>
    where
        ClientDirection: Future<Output = ClientLoopExit>,
        BackendDirection: Future<Output = Result<BackendClose, BackendError>>,
    {
        let mut client_future = Box::pin(client_direction);
        let mut backend_future = Box::pin(backend_direction);
        let mut client_exit = None;
        let mut backend_result = None;
        match select(client_future.as_mut(), backend_future.as_mut()).await {
            Either::Left((exit, _backend)) => client_exit = Some(exit),
            Either::Right((result, _client)) => backend_result = Some(result),
        }
        let client_settled = client_exit.is_some();
        let mut backend_settled = backend_result.is_some();
        let mut failure = None;
        // A disconnected client is gone: LiteLLM returns before it looks at the backend's own close
        if client_exit != Some(ClientLoopExit::ClientDisconnected) {
            let outcome = match backend_result {
                Some(result) => result,
                None => {
                    backend_settled = true;
                    backend_future.as_mut().await
                }
            };
            match outcome {
                Ok(close) => self.close_client(&close).await,
                Err(error) => failure = Some(error),
            }
        }
        cancellation.cancel();
        // A direction that ignored cancellation would hold the session open, so both are joined here rather than detached
        if !client_settled {
            let _joined_client = client_future.as_mut().await;
        }
        if !backend_settled {
            let _joined_backend = backend_future.as_mut().await;
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    // One sender around the same state, provider and callbacks for every step that writes to a socket
    pub(crate) fn sender(&mut self) -> RealtimeSender<'_, Config, CallbackError> {
        RealtimeSender {
            state: &mut *self.state,
            provider: self.provider,
            guardrails: &self.callbacks,
        }
    }

    // LiteLLM's redact_internal_details_from_client_message with secret redaction disabled by config
    pub(crate) fn redact(&self, text: &str) -> String {
        match self.redactor {
            Some(redactor) => redactor.redact_internal_details(text),
            None => text.to_owned(),
        }
    }
}
