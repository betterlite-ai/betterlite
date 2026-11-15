use std::sync::Arc;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::gcs_client::gcs_storage_auth::environment_source::EnvironmentSource;
use crate::gcs_client::gcs_storage_auth::initialization_behaviors::initialize;
use crate::gcs_logger::auth::GoogleCloudAuth;
use crate::gcs_logger::environment::CatalogEnvironment;
use crate::gcs_logger::gcs_bucket_logging::initialization_behaviors::start;
use crate::gcs_logger::gcs_bucket_logging::{GcsBucketLogger, GcsDeps};
use crate::gcs_logger::runtime::RandomIds;
use crate::gcs_logger::transport::PooledGcsClient;
use crate::prometheus_query::clock::SystemClock;

mod tests;

// LiteLLM's GCSBucketLogger() reads GCS_BUCKET_NAME and the mock settings in its parent's __init__
pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let environment: Arc<dyn EnvironmentSource> =
        Arc::new(CatalogEnvironment::new(Arc::clone(&inputs.env)));
    let base = initialize(
        environment,
        None,
        GoogleCloudAuth::new(),
        PooledGcsClient {
            client: inputs.http.clone(),
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let logger: Arc<GcsBucketLogger<GoogleCloudAuth, PooledGcsClient>> = start(
        base,
        GcsDeps {
            clock: Arc::new(SystemClock),
            ids: Arc::new(RandomIds),
            tasks: Some(inputs.tracker.clone()),
            shutdown: inputs.shutdown.clone(),
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let callback: Arc<dyn CallbackLogger> = logger;
    Ok(callback)
}
