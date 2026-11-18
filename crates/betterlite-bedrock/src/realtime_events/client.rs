use async_trait::async_trait;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ClientError {
    pub message: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

// RealtimeClientWebSocket's transport surface; the bridge owns the concrete socket behind it
#[async_trait]
pub trait RealtimeClientWebSocket: Send + Sync {
    async fn receive_text(&self) -> Result<String, ClientError>;

    async fn send_text(&self, data: String) -> Result<(), ClientError>;

    async fn close(&self, code: u16, reason: Option<String>);
}

// _client_messages as a pull-based iterator: the first frame is the preface the bridge already holds
pub struct ClientMessages<'a> {
    pub client: &'a dyn RealtimeClientWebSocket,
    pub initial: Option<String>,
    pub redactor: &'a Redactor,
    pub done: bool,
}

impl ClientMessages<'_> {
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
