mod ack;
mod close_client;
pub mod error;
mod lifetime;
mod rewrite;
mod session;

#[cfg(test)]
mod tests;

use std::convert::Infallible;
use std::error::Error;
use std::future::Future;

use betterlite_redact::secret::Redactor;
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
    // LiteLLM's _receive_client_message: whatever the client socket raises means the client is gone
    pub async fn receive_client_message(&mut self) -> Option<String> {
        ack::receive(self.state.client.as_deref_mut()).await
    }

    pub async fn client_ack_messages(&mut self) -> ClientLoopExit {
        ack::run(self).await
    }

    // LiteLLM's _close_client: one failed attempt is logged and swallowed, so a failed notice also skips the close
    pub async fn close_client(&mut self, close: &BackendClose) {
        let message = self.redact(&close.message());
        let reason = self.redact(&close.reason);
        close_client::relay(
            self.state.client.as_deref_mut(),
            close.code,
            &message,
            &reason,
        )
        .await;
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
        lifetime::run(self, client_direction, backend_direction, cancellation).await
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
