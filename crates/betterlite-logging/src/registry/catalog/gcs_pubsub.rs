use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;

use super::{CatalogError, CatalogInputs};
use crate::batch::logger::{BATCH_SIZE_VARIABLE, BatchSettings, FLUSH_INTERVAL_VARIABLE};
use crate::callback_logger::CallbackLogger;
use crate::gcs_logger::auth::GoogleCloudAuth;
use crate::gcs_pubsub::{GcsPubSubDeps, GcsPubSubLogger, Overrides};
use crate::http_analytics_loggers::delivery::PooledDelivery;
use crate::private_json::error::PrivateJsonError;
use crate::spend_metadata::SpendMetadataError;

pub(crate) mod flush;
pub(crate) mod payload;
mod publish;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error("the {field} timestamp is out of range")]
    Time { field: &'static str },
    #[error("the {record} could not be written as JSON")]
    Json {
        record: &'static str,
        #[source]
        source: PrivateJsonError,
    },
    #[error("the spend log record could not be written as JSON")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Metadata(#[from] SpendMetadataError),
}

// LiteLLM's GcsPubSubLogger() reads GCS_PUBSUB_PROJECT_ID and GCS_PUBSUB_TOPIC_ID in its __init__
pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let logger = GcsPubSubLogger::new(
        inputs.env.as_ref(),
        Overrides::default(),
        GcsPubSubDeps {
            auth: GoogleCloudAuth::default(),
            delivery: PooledDelivery {
                client: inputs.http.clone(),
            },
            project: payload::spend_log_payload,
            batch: batch_settings(inputs.env.as_ref())?,
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let process = Arc::new(logger);
    flush::spawn_flush_loop(&process, inputs.tracker.clone(), inputs.shutdown.clone());
    let callback: Arc<dyn CallbackLogger> = process;
    Ok(callback)
}

fn batch_settings(environment: &dyn EnvironmentLookup) -> Result<BatchSettings, CatalogError> {
    let mut values = BTreeMap::new();
    for name in [BATCH_SIZE_VARIABLE, FLUSH_INTERVAL_VARIABLE] {
        if let Some(text) = environment.lookup(name) {
            values.insert(name.to_owned(), text);
        }
    }
    BatchSettings::from_environment(&values).map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })
}
