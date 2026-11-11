use std::sync::Arc;

use betterlite_config::sections::callback_settings::names::CallbackName;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::s3_batch::credentials::ConfiguredKeys;
use crate::s3_batch::flush::SystemMonotonic;
use crate::s3_batch::logger::{S3BatchDeps, S3BatchLogger};
use crate::s3_batch::ndjson_batch_file::UuidBatchIds;
use crate::s3_transport::signing::SystemClock;
use crate::s3_transport::transport::HttpS3Client;
use crate::s3_v2::parameters::resolve_s3_parameters;
use crate::s3_v2::source::Source;

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let source = parameter_source(inputs)?;
    let parameters =
        resolve_s3_parameters::<()>(&Source::default(), &source, None, inputs.env.as_ref());
    let keys = ConfiguredKeys {
        access_key_id: parameters.connection.credentials.aws_access_key_id.clone(),
        secret_access_key: parameters
            .connection
            .credentials
            .aws_secret_access_key
            .clone(),
        session_token: parameters.connection.credentials.aws_session_token.clone(),
        environment: Arc::clone(&inputs.env),
    };
    let logger: Arc<dyn CallbackLogger> = S3BatchLogger::new(
        parameters,
        S3BatchDeps {
            client: Arc::new(HttpS3Client {
                client: inputs.http.clone(),
            }),
            credentials: Arc::new(keys),
            clock: Arc::new(SystemClock),
            monotonic: Arc::new(SystemMonotonic::default()),
            batch_ids: Arc::new(UuidBatchIds),
            environment: Arc::clone(&inputs.env),
            throttle_observer: None,
            // B1729 loads litellm_settings.cold_storage_custom_logger; until then no logger is cold storage
            cold_storage_logger: None,
            tasks: Some(inputs.tracker.clone()),
            shutdown: inputs.shutdown.clone(),
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    Ok(logger)
}

// the scalar keys a proxy config keeps under callback_settings.s3_v2 carry the s3_callback_params bag
fn parameter_source(inputs: &CatalogInputs) -> Result<Source<()>, CatalogError> {
    let name = CallbackName::S3V2.as_str();
    let Some(entry) = inputs.callback_settings.get(name) else {
        return Ok(Source::default());
    };
    let text =
        serde_json::to_string(&entry.exposed_variables()).map_err(|source| CatalogError::Init {
            message: source.to_string(),
        })?;
    let mut deserializer = serde_json::Deserializer::from_str(&text);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|error| CatalogError::Init {
        message: format!(
            "callback_settings.{name}.{}: {}",
            error.path(),
            error.inner()
        ),
    })
}
