use std::sync::Arc;

use betterlite_config::sections::callback_settings::CallbackEntry;
use betterlite_config::sections::callback_settings::names::CallbackName;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::http_analytics_loggers::delivery::PooledDelivery;
use crate::legacy_dynamodb::DynamoDbLogger;
use crate::legacy_dynamodb::client::HttpDynamoClient;
use crate::legacy_dynamodb::settings;
use crate::sqs_logging::clock::SystemClock;
use crate::sqs_logging::credentials::{CredentialParams, EnvironmentCredentials};

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let name = CallbackName::Dynamodb.as_str();
    let params = inputs
        .callback_settings
        .get(name)
        .map(CallbackEntry::exposed_variables)
        .unwrap_or_default();
    let resolved =
        settings::resolve(&params, inputs.env.as_ref()).map_err(|error| CatalogError::Init {
            message: error.to_string(),
        })?;
    let client = HttpDynamoClient {
        delivery: PooledDelivery {
            client: inputs.http.clone(),
        },
        credentials: Arc::new(EnvironmentCredentials {
            environment: Arc::clone(&inputs.env),
        }),
        params: CredentialParams::default(),
        clock: Arc::new(SystemClock),
        endpoint: settings::endpoint_url(&resolved.region_name),
        region: resolved.region_name.clone(),
    };
    Ok(Arc::new(DynamoDbLogger {
        settings: resolved,
        client,
    }))
}
