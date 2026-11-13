pub mod delivery;
pub mod error;
pub mod retry;
mod tests;
pub mod transport;
use self::{retry::RetrySleeper, transport::PostTransport};
use super::generic_settings::GenericApiSettings;
use crate::callback_logger::{CallbackLogger, error::LoggerError, event::LoggerEvent};
use crate::fields::payload::StandardLoggingPayload;
use async_trait::async_trait;
use betterlite_models::decisions::Json;
use std::mem;
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
pub type IdGenerator = fn() -> String;
#[must_use]
pub fn uuid_id() -> String {
    // the v1 payload is the only place an id is invented, as LiteLLM does with uuid4
    uuid::Uuid::new_v4().to_string()
}
pub struct LoggerDeps {
    pub transport: Arc<dyn PostTransport>,
    pub sleeper: Arc<dyn RetrySleeper>,
    pub ids: IdGenerator,
}
pub struct GenericApiLogger {
    settings: GenericApiSettings,
    transport: Arc<dyn PostTransport>,
    sleeper: Arc<dyn RetrySleeper>,
    ids: fn() -> String,
    use_v1: bool,
    batch_size: usize,
    queue: Mutex<Vec<Json>>,
    flush_lock: AsyncMutex<()>,
}
impl GenericApiLogger {
    pub fn new(
        settings: GenericApiSettings,
        deps: LoggerDeps,
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
            transport: deps.transport,
            sleeper: deps.sleeper,
            ids: deps.ids,
            use_v1,
            batch_size,
            queue: Mutex::new(Vec::new()),
            flush_lock: AsyncMutex::new(()),
        }
    }
    pub async fn record(&self, kind: ApiEventType, payload: &StandardLoggingPayload) {
        let listed = self
            .settings
            .event_types
            .as_ref()
            .is_none_or(|types| types.iter().any(|name| name == kind.name()));
        if !listed {
            return;
        }
        let queued = if self.use_v1 {
            delivery::v1_entry(payload, &(self.ids)())
        } else {
            delivery::standard_entry(payload)
        };
        let entry = match queued {
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
            let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
            mem::take(&mut *queue)
        };
        if batch.is_empty() {
            return;
        }
        tracing::debug!(
            events = batch.len(),
            format = %self.settings.log_format,
            "Generic API Logger - about to flush events"
        );
        // draining first means a cancelled or failed send cannot post the batch a second time
        if let Err(error) = delivery::deliver(self, &batch).await {
            tracing::error!(%error, "Generic API Logger Error sending batch");
        }
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
        self.record(ApiEventType::Success, event.payload.as_ref())
            .await;
        Ok(())
    }
    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.record(ApiEventType::Failure, event.payload.as_ref())
            .await;
        Ok(())
    }
}
