use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::task::TaskTracker;

use super::payload::success_event;
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::helicone::initialization::HeliconeLogger;

pub(super) struct HeliconeAdapter {
    pub(super) logger: Arc<HeliconeLogger>,
    pub(super) tracker: TaskTracker,
}

// HeliconeLogger.log_success is reached only from the sync success handler body; the async handler has no helicone branch.
#[async_trait]
impl CallbackLogger for HeliconeAdapter {
    fn callback_name(&self) -> &'static str {
        "helicone"
    }

    fn log_success_event(&self, event: &LoggerEvent) -> Result<(), LoggerError> {
        let call = success_event(&event.payload)?;
        let logger = Arc::clone(&self.logger);
        let _spawned = self.tracker.spawn(async move {
            logger.log_success(&call).await;
        });
        Ok(())
    }
}
