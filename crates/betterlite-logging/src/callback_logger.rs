pub mod error;
pub mod event;
pub mod resolver;
mod tests;

use std::sync::Arc;

use async_trait::async_trait;

use error::LoggerError;
use event::LoggerEvent;

use crate::audit::payload::StandardAuditLogPayload;
use crate::cold_storage_key::ColdStorageTarget;
use crate::exporter_options::failure_counter::CallbackFailureCounter;
use crate::mcp_tool_hook::types::{McpPostCallResponse, McpToolHookCall};

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

    // the deprecated constructor flag; false redacts the messages and the response for this logger
    fn message_logging(&self) -> bool {
        true
    }

    // only the s3_v2 logger has a path and a partition granularity a cold-storage key can follow
    fn cold_storage_target(&self) -> Option<ColdStorageTarget> {
        None
    }

    // only the Prometheus logger has increment_callback_logging_failure, so a failed call counts there
    fn failure_counter(&self) -> Option<&dyn CallbackFailureCounter> {
        None
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

    // CustomLogger.async_post_mcp_tool_call_hook answers None, so a logger opts in by overriding it
    async fn async_post_mcp_tool_call_hook(
        &self,
        _call: McpToolHookCall<'_>,
    ) -> Result<Option<McpPostCallResponse>, LoggerError> {
        Ok(None)
    }

    // CustomLogger.async_log_audit_log_event is a pass by default; the Rust name drops one "log" because B4002's AzureSentinelLogger keeps LiteLLM's as an inherent method (clippy::same_name_method)
    async fn async_log_audit_event(
        &self,
        _audit_log: &StandardAuditLogPayload,
    ) -> Result<(), LoggerError> {
        Ok(())
    }
}
