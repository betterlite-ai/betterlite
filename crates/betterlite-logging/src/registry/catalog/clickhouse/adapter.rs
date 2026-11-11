use std::sync::Arc;

use async_trait::async_trait;

use super::payload::callback_kwargs;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::clickhouse::clickhouse_logging::context::LensAnalysis;
use crate::clickhouse::clickhouse_logging::spend_logger::{CallbackEvent, ClickHouseSpendLogger};
use crate::clickhouse::clickhouse_logging::storage::ClickHouseStorage;

pub(super) struct ClickHouseAdapter<Storage: ClickHouseStorage> {
    pub(super) logger: ClickHouseSpendLogger<Storage>,
    pub(super) lens: LensAnalysis,
}

fn delivery_error(error: &serde_json::Error) -> LoggerError {
    LoggerError::Delivery {
        callback: "clickhouse".to_owned(),
        message: error.to_string(),
    }
}

// the success and failure hooks are async in LiteLLM, and the batch logger never waits on ClickHouse
#[async_trait]
impl<Storage: ClickHouseStorage + 'static> CallbackLogger for ClickHouseAdapter<Storage> {
    fn callback_name(&self) -> &'static str {
        "clickhouse"
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let kwargs = callback_kwargs(&event.payload).map_err(|error| delivery_error(&error))?;
        let call = CallbackEvent {
            kwargs: &kwargs,
            // B190 redacts the stored payload before the event is built, so the projection keeps what it got
            redact: false,
            lens: &self.lens,
        };
        self.logger.async_log_success_event(&call).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        let kwargs = callback_kwargs(&event.payload).map_err(|error| delivery_error(&error))?;
        let call = CallbackEvent {
            kwargs: &kwargs,
            redact: false,
            lens: &self.lens,
        };
        self.logger.async_log_failure_event(&call).await;
        Ok(())
    }
}
