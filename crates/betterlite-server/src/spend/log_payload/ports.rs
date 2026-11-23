use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::spend_bodies::support::Fields;

use super::dates::DetailDates;

// AdditionalLoggingUtils.get_request_response_payload: a callback that serves a whole logged request by its id
#[async_trait]
pub trait PayloadLogger: Send + Sync {
    async fn request_response_payload(
        &self,
        request_id: &str,
        window: &DetailDates,
    ) -> Result<Option<BTreeMap<String, JsonData>>, LoggerError>;
}

// CustomLogger.get_proxy_server_request_from_cold_storage_with_object_key: the stored object under a key
#[async_trait]
pub trait ColdStorageReader: Send + Sync {
    async fn proxy_server_request(&self, object_key: &str) -> Result<Option<Fields>, LoggerError>;
}

// litellm.logging_callback_manager as a request sees it, since callbacks change at runtime
pub trait ActiveLoggers: Send + Sync {
    // get_active_additional_logging_utils_from_custom_logger, in registry order
    fn payload_loggers(&self) -> Vec<Arc<dyn PayloadLogger>>;
    // get_active_custom_logger_for_callback_name: the first active logger registered under the name
    fn cold_storage_logger(&self, callback_name: &str) -> Option<Arc<dyn ColdStorageReader>>;
}
