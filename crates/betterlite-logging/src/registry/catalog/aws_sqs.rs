use std::sync::Arc;

use betterlite_config::sections::callback_settings::CallbackEntry;
use betterlite_config::sections::callback_settings::names::CallbackName;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::http_analytics_loggers::delivery::PooledDelivery;
use crate::sqs_logging::clock::SystemClock;
use crate::sqs_logging::credentials::EnvironmentCredentials;
use crate::sqs_logging::flush::spawn_flush_loop;
use crate::sqs_logging::health::DummyHealthPayload;
use crate::sqs_logging::settings::{Overrides, Settings};
use crate::sqs_logging::{SqsDeps, SqsLogger};

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let name = CallbackName::AwsSqs.as_str();
    let params = inputs
        .callback_settings
        .get(name)
        .map(CallbackEntry::exposed_variables)
        .unwrap_or_default();
    let settings =
        Settings::resolve(&params, Overrides::default(), inputs.env.as_ref()).map_err(|error| {
            CatalogError::Init {
                message: error.to_string(),
            }
        })?;
    let logger = SqsLogger::new(
        settings,
        SqsDeps {
            delivery: PooledDelivery {
                client: inputs.http.clone(),
            },
            credentials: Arc::new(EnvironmentCredentials {
                environment: Arc::clone(&inputs.env),
            }),
            clock: Arc::new(SystemClock),
            health_payload: Arc::new(DummyHealthPayload),
            tasks: inputs.tracker.clone(),
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let process = Arc::new(logger);
    spawn_flush_loop(&process, inputs.shutdown.clone());
    Ok(process)
}
