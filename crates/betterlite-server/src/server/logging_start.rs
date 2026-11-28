use std::sync::Arc;

use betterlite_config::types::Config;
use betterlite_error::normalize::{NormalizeError, Normalizer};
use betterlite_logging::logging_metadata::base64::{Base64Error, Base64Truncator};
use betterlite_logging::logging_service_events::clock::SystemClock;
use betterlite_logging::logging_service_events::sink::{ServiceEventSink, TracingSink};
use betterlite_logging::logging_service_events::wiring::{
    LoggingInputs, LoggingWiring, SuccessInputs,
};
use betterlite_logging::logging_success::ports::{StreamAssembler, SuccessClock};
use betterlite_logging::payload::builder::PayloadBuilderConfig;
use betterlite_logging::payload::deps::{PayloadSettings, PayloadSources, TagConfig};
use betterlite_logging::registry::CallbackRegistry;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_router::prompt_dispatch::known_callbacks::KNOWN_CALLBACKS;
use thiserror::Error;

use super::ServerError;

mod tests;

#[derive(Debug, Error)]
pub enum SuccessInputsError {
    #[error("the payload redaction settings could not be built: {source}")]
    Redaction {
        #[source]
        source: RedactionError,
    },
    #[error("the payload normalizer could not be built: {source}")]
    Normalizer {
        #[source]
        source: NormalizeError,
    },
    #[error("MAX_BASE64_LENGTH_FOR_LOGGING is not a usable placeholder length: {source}")]
    Truncator {
        #[source]
        source: Base64Error,
    },
}

pub(super) fn start_logging(
    config: &Config,
    registry: CallbackRegistry,
    stream_assembler: Arc<dyn StreamAssembler>,
) -> Result<LoggingWiring, ServerError> {
    let clock: Arc<dyn SuccessClock> = Arc::new(SystemClock);
    let sink: Arc<dyn ServiceEventSink> = Arc::new(TracingSink);
    Ok(LoggingWiring::start(LoggingInputs {
        settings: &config.sections.service_callback,
        clock,
        sink,
        registry,
        stream_assembler,
        success: success_inputs(config)?,
    })?)
}

fn success_inputs(config: &Config) -> Result<SuccessInputs, SuccessInputsError> {
    Ok(SuccessInputs {
        payload: PayloadBuilderConfig {
            sources: Arc::new(PayloadSources::default()),
            settings: PayloadSettings {
                tag_config: TagConfig::default(),
                request_correlation_in_logs: false,
                log_client_error_tracebacks: config
                    .sections
                    .auth_errors
                    .log_client_error_tracebacks,
            },
            redactor: Arc::new(
                Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
                    .map_err(|source| SuccessInputsError::Redaction { source })?,
            ),
            normalizer: Arc::new(
                Normalizer::new().map_err(|source| SuccessInputsError::Normalizer { source })?,
            ),
            truncator: Base64Truncator::new(
                usize::try_from(config.sections.prompt_affinity.maximum_base64_length).unwrap_or(0),
            )
            .map_err(|source| SuccessInputsError::Truncator { source })?,
            privacy: config.sections.logging_privacy,
        },
        excluded_fields: config.sections.payload_exclusions.excluded_fields.clone(),
        known_callback: Arc::new(|name: &str| KNOWN_CALLBACKS.contains(&name)),
    })
}
