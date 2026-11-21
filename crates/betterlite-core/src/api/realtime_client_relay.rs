mod ack;
mod close_client;
pub mod error;
mod rewrite;
mod session;

mod tests;

use std::convert::Infallible;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use betterlite_redact::secret::Redactor;
use futures_util::future::{Either, select};
use tokio_util::sync::CancellationToken;

use crate::api::realtime_contracts::close::{BackendClose, ClientLoopExit};
use crate::api::realtime_contracts::config::BaseRealtimeConfig;
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
    #[must_use]
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

    #[must_use]
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
        ClientDirection: Future<Output = ClientLoopExit> + Send,
        BackendDirection: Future<Output = Result<BackendClose, BackendError>> + Send,
        BackendError: Send,
    {
        let mut client_future = Box::pin(client_direction);
        let mut backend_future = Box::pin(backend_direction);
        let (client_exit, backend_result) =
            match select(client_future.as_mut(), backend_future.as_mut()).await {
                Either::Left((exit, _backend)) => (Some(exit), None),
                Either::Right((result, _client)) => (None, Some(result)),
            };
        let failure = self
            .settle(client_exit, backend_result, &mut backend_future)
            .await?;
        cancellation.cancel();
        // A direction that ignored cancellation would hold the session open, so both are joined here rather than detached
        if client_exit.is_none() {
            let _joined_client = client_future.as_mut().await;
        }
        if client_exit == Some(ClientLoopExit::ClientDisconnected) {
            let _joined_backend = backend_future.as_mut().await;
        }
        failure.map_or(Ok(()), Err)
    }

    // The first direction to finish decides the close, unless the client has already disconnected
    async fn settle<BackendDirection, BackendError>(
        &mut self,
        client_exit: Option<ClientLoopExit>,
        backend_result: Option<Result<BackendClose, BackendError>>,
        backend_future: &mut Pin<Box<BackendDirection>>,
    ) -> Result<Option<BackendError>, BackendError>
    where
        BackendDirection: Future<Output = Result<BackendClose, BackendError>> + Send,
        BackendError: Send,
    {
        if client_exit == Some(ClientLoopExit::ClientDisconnected) {
            return Ok(None);
        }
        let outcome = match backend_result {
            Some(result) => result,
            None => backend_future.as_mut().await,
        };
        match outcome {
            Ok(close) => {
                self.close_client(&close).await;
                Ok(None)
            }
            Err(error) => Ok(Some(error)),
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
        self.redactor.map_or_else(
            || text.to_owned(),
            |redactor| redactor.redact_internal_details(text),
        )
    }
}
