mod tests;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::callback_settings::{CallbackEntry, names::CallbackName};

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::zerobus::logger::connection::EnvironmentSecrets;
use crate::zerobus::logger::params::configured_params;
use crate::zerobus::logger::payload::RedactionSettings;
use crate::zerobus::logger::{ZerobusDeps, ZerobusLogger};

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let name = CallbackName::Zerobus.as_str();
    let variables = inputs
        .callback_settings
        .get(name)
        .map(CallbackEntry::exposed_variables);
    let params = configured_params(variables.as_ref()).map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let settings = &inputs.config.sections;
    let deps = ZerobusDeps {
        params,
        client: None,
        secrets: Arc::new(EnvironmentSecrets {
            environment: Arc::clone(&inputs.env),
        }),
        http: inputs.http.clone(),
        clock: Arc::new(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
        }),
        redaction: RedactionSettings {
            turn_off_message_logging: settings.logging_privacy.turn_off_message_logging,
            excluded_fields: settings.payload_exclusions.excluded_fields.clone(),
        },
        tasks: inputs.tracker.clone(),
        shutdown: inputs.shutdown.clone(),
        start_periodic_flush: true,
        max_queue_size: None,
    };
    let logger = ZerobusLogger::new(deps).map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    Ok(Arc::new(logger))
}
