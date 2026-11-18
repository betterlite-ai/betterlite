use std::error::Error as StdError;

use async_trait::async_trait;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ClientError<E> {
    pub message: String,
    #[source]
    pub source: Option<E>,
}

// RealtimeClientWebSocket's transport surface; the bridge owns the concrete socket behind it
#[async_trait]
pub trait RealtimeClientWebSocket: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn receive_text(&self) -> Result<String, ClientError<Self::Error>>;

    async fn send_text(&self, data: String) -> Result<(), ClientError<Self::Error>>;

    async fn close(&self, code: u16, reason: Option<String>);
}

// _client_messages as a pull-based iterator: the first frame is the preface the bridge already holds
pub struct ClientMessages<'client, E: StdError + Send + Sync + 'static> {
    pub client: &'client dyn RealtimeClientWebSocket<Error = E>,
    pub initial: Option<String>,
    pub redactor: &'client Redactor,
    pub done: bool,
}

impl<E: StdError + Send + Sync + 'static> ClientMessages<'_, E> {
    // A receive failure ends the generator: LiteLLM's iterator returns and never receives again
    pub async fn next(&mut self) -> Option<String> {
        if self.done {
            return None;
        }
        if let Some(initial) = self.initial.take() {
            return Some(initial);
        }
        match self.client.receive_text().await {
            Ok(text) => Some(text),
            Err(error) => {
                self.done = true;
                tracing::debug!(
                    "Client to Bedrock forwarding ended: {}",
                    self.redactor.redact_string(&error.message)
                );
                None
            }
        }
    }
}
