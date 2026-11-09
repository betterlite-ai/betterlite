pub mod delivery;
pub mod error;
pub mod retry;
mod tests;
pub mod transport;
use self::{error::ApiError, retry::RetrySleeper, transport::PostTransport};
use super::generic_settings::GenericApiSettings;
use crate::callback_logger::{CallbackLogger, error::LoggerError, event::LoggerEvent};
use crate::fields::payload::StandardLoggingPayload;
use async_trait::async_trait;
use betterlite_models::decisions::Json;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::Mutex as AsyncMutex;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiEventType {
    Success,
    Failure,
}
impl ApiEventType {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Success => "llm_api_success",
            Self::Failure => "llm_api_failure",
        }
    }
}
pub struct GenericApiLogger {
    settings: GenericApiSettings,
    transport: Arc<dyn PostTransport>,
    sleeper: Arc<dyn RetrySleeper>,
    use_v1: bool,
    batch_size: usize,
    queue: Mutex<Vec<Json>>,
    flush_lock: AsyncMutex<()>,
}
impl GenericApiLogger {
    pub fn new(
        settings: GenericApiSettings,
        transport: Arc<dyn PostTransport>,
        sleeper: Arc<dyn RetrySleeper>,
        use_v1: bool,
        batch_size: usize,
    ) -> Self {
        // LiteLLM prints the endpoint and headers here, and a config header can hold a secret
        tracing::debug!(
            callback_name = ?settings.callback_name,
            log_format = %settings.log_format,
            event_types = ?settings.event_types,
            "in init GenericAPILogger"
        );
        Self {
            settings,
            transport,
            sleeper,
            use_v1,
            batch_size,
            queue: Mutex::new(Vec::new()),
            flush_lock: AsyncMutex::new(()),
        }
    }

    fn wants(&self, kind: ApiEventType) -> bool {
        let Some(types) = &self.settings.event_types else {
            return true;
        };
        types.iter().any(|name| name == kind.name())
    }

    fn entry(&self, payload: Option<&StandardLoggingPayload>) -> Result<Json, ApiError> {
        if self.use_v1 {
            let standard = payload.ok_or(ApiError::MissingPayload)?;
            return delivery::v1_entry(standard);
        }
        payload.map_or_else(|| Ok(Json::Null), delivery::standard_entry)
    }

    pub async fn record(&self, kind: ApiEventType, payload: Option<&StandardLoggingPayload>) {
        if !self.wants(kind) {
            return;
        }
        let entry = match self.entry(payload) {
            Ok(entry) => entry,
            Err(error) => {
                tracing::error!(%error, "Generic API Logger Error");
                return;
            }
        };
        let depth = {
            let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            queue.push(entry);
            queue.len()
        };
        if depth >= self.batch_size {
            self.send_batch().await;
        }
    }

    pub async fn send_batch(&self) {
        let _flush = self.flush_lock.lock().await;
        let batch = {
            let queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            queue.clone()
        };
        if batch.is_empty() {
            return;
        }
        tracing::debug!(
            events = batch.len(),
            format = %self.settings.log_format,
            "Generic API Logger - about to flush events"
        );
        // LiteLLM's finally clears the queue, so a failed batch is dropped rather than kept
        if let Err(error) = delivery::deliver(self, &batch).await {
            tracing::error!(%error, "Generic API Logger Error sending batch");
        }
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}
#[async_trait]
impl CallbackLogger for GenericApiLogger {
    fn callback_name(&self) -> &str {
        self.settings
            .callback_name
            .as_deref()
            .unwrap_or("generic_api")
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record(ApiEventType::Success, Some(event.payload.as_ref()))
            .await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record(ApiEventType::Failure, Some(event.payload.as_ref()))
            .await;
        Ok(())
    }
}
