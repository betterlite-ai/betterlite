use std::sync::Arc;

use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::spend_bodies::support::Fields;

use super::ports::{ActiveLoggers, ColdStorageReader};

pub struct ColdStorageHandler {
    // a reader handed in by the caller wins over the configured callback
    pub injected: Option<Arc<dyn ColdStorageReader>>,
    // litellm_settings.cold_storage_custom_logger
    pub selected: Option<String>,
    pub loggers: Arc<dyn ActiveLoggers>,
}

impl ColdStorageHandler {
    // get_proxy_server_request_from_cold_storage_with_object_key: no selected or active logger reads nothing
    pub async fn proxy_server_request(
        &self,
        object_key: &str,
    ) -> Result<Option<Fields>, LoggerError> {
        if let Some(injected) = &self.injected {
            return injected.proxy_server_request(object_key).await;
        }
        let Some(name) = self.selected.as_deref() else {
            return Ok(None);
        };
        let Some(active) = self.loggers.cold_storage_logger(name) else {
            tracing::warn!(
                callback = name,
                "No active custom logger found for the cold storage callback"
            );
            return Ok(None);
        };
        active.proxy_server_request(object_key).await
    }
}
