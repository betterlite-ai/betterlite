pub mod error;
pub mod event;
pub mod resolver;
mod tests;

use std::sync::Arc;

use async_trait::async_trait;

use error::LoggerError;
use event::LoggerEvent;

// CustomLogger's own body is pass, so a logger overrides only the events it handles
#[async_trait]
pub trait CallbackLogger: Send + Sync {
    fn callback_name(&self) -> &str;

    fn turn_off_message_logging(&self) -> bool {
        false
    }

    fn redacts_messages_itself(&self) -> bool {
        false
    }

    fn log_success_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        Ok(())
    }

    fn log_failure_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        Ok(())
    }

    fn log_stream_event(&self, _event: &LoggerEvent) -> Result<(), LoggerError> {
        Ok(())
    }

    async fn async_log_success_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        Ok(())
    }

    async fn async_log_failure_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        Ok(())
    }

    async fn async_log_stream_event(&self, _event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        Ok(())
    }
}
